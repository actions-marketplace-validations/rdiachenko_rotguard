//! Fetching URLs politely: one request in flight per host, spaced starts, bounded hosts.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use reqwest::header::{LOCATION, RETRY_AFTER};
use reqwest::redirect::Policy;
use reqwest::{Client, Response, StatusCode};
use serde::Serialize;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{Instant, sleep_until};
use url::Url;

use crate::config::Config;

/// Most redirects followed before giving up.
const MAX_REDIRECTS: usize = 10;
/// Most body bytes kept.
const MAX_BODY: usize = 256 * 1024;
/// Most hosts with a request in flight at once.
const MAX_HOSTS_IN_FLIGHT: usize = 8;

/// One response in a redirect chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hop {
    pub url: Url,
    pub status: u16,
}

/// The outcome of fetching a URL and following its redirects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// Every response seen, in order; the last is the final one.
    pub chain: Vec<Hop>,
    /// The final response's body, cut at 256 KB.
    pub body: Vec<u8>,
    /// `Retry-After` of the final response, when given in seconds.
    pub retry_after: Option<Duration>,
}

/// Why a URL could not be fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    Dns,
    Timeout,
    Tls,
    Connect,
    /// A redirect without a usable `Location`.
    NoLocation(Vec<Hop>),
    /// A redirect to a URL already in the chain.
    Loop(Vec<Hop>),
    /// More than 10 redirects.
    TooManyHops(Vec<Hop>),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Dns => "dns",
            Self::Timeout => "timeout",
            Self::Tls => "tls",
            Self::Connect => "connection",
            Self::NoLocation(_) => "no location",
            Self::Loop(_) => "redirect loop",
            Self::TooManyHops(_) => "too many hops",
        })
    }
}

impl Error for FetchError {}

/// Fetches URLs through per-host gates.
#[derive(Debug)]
pub struct Fetcher {
    client: Client,
    interval: Duration,
    gates: StdMutex<HashMap<String, Arc<Mutex<Option<Instant>>>>>,
    hosts: Semaphore,
}

impl Fetcher {
    /// Builds the client once from the config's User-Agent and timeout.
    ///
    /// # Panics
    /// If the TLS backend cannot initialise, which is a broken build or platform. A
    /// `user_agent` from a parsed `Config` is already checked as a valid header value.
    #[must_use]
    pub fn new(cfg: &Config) -> Fetcher {
        let client = Client::builder()
            .redirect(Policy::none())
            .timeout(cfg.timeout)
            .user_agent(cfg.user_agent.as_str())
            .build()
            .expect("user_agent is validated by config::parse; the TLS backend is built in");
        Fetcher {
            client,
            interval: cfg.host_interval,
            gates: StdMutex::new(HashMap::new()),
            hosts: Semaphore::new(MAX_HOSTS_IN_FLIGHT),
        }
    }

    /// GETs `url`, following redirects by hand and recording every hop.
    pub async fn fetch(&self, url: &Url) -> Result<Fetched, FetchError> {
        let mut chain: Vec<Hop> = Vec::new();
        let mut current = url.clone();
        loop {
            let (hop, outcome) = self.request(&current).await?;
            chain.push(hop);
            let next = match outcome {
                Outcome::Final { body, retry_after } => {
                    return Ok(Fetched {
                        chain,
                        body,
                        retry_after,
                    });
                }
                Outcome::Redirect(Some(next)) => next,
                Outcome::Redirect(None) => return Err(FetchError::NoLocation(chain)),
            };
            if chain.iter().any(|hop| hop.url == next) {
                return Err(FetchError::Loop(chain));
            }
            if chain.len() > MAX_REDIRECTS {
                return Err(FetchError::TooManyHops(chain));
            }
            current = next;
        }
    }

    /// One request through its host's gate and the global permit.
    ///
    /// Returns the hop and what followed it.
    async fn request(&self, url: &Url) -> Result<(Hop, Outcome), FetchError> {
        let Some(host) = url.host_str() else {
            return Err(FetchError::Connect);
        };
        let gate = self.gate(host);
        // Gate first, permit second: requests queued on one busy host must not hold permits
        // that other hosts need.
        let mut last_start = gate.lock().await;
        if let Some(last) = *last_start {
            sleep_until(last + self.interval).await;
        }
        let _permit = self
            .hosts
            .acquire()
            .await
            .expect("the semaphore is never closed");
        *last_start = Some(Instant::now());

        let response = self.client.get(url.clone()).send().await.map_err(map_err)?;
        let status = response.status();
        let hop = Hop {
            url: url.clone(),
            status: status.as_u16(),
        };
        if is_redirect(status) {
            let target = response
                .headers()
                .get(LOCATION)
                .map(|value| String::from_utf8_lossy(value.as_bytes()))
                .and_then(|location| url.join(location.trim()).ok())
                .filter(|next| {
                    matches!(next.scheme(), "http" | "https") && next.host_str().is_some()
                });
            return Ok((hop, Outcome::Redirect(target)));
        }
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = read_capped(response).await?;
        Ok((hop, Outcome::Final { body, retry_after }))
    }

    fn gate(&self, host: &str) -> Arc<Mutex<Option<Instant>>> {
        let mut gates = self.gates.lock().expect("no code panics holding this lock");
        Arc::clone(gates.entry(host.to_owned()).or_default())
    }
}

/// What a response led to.
enum Outcome {
    /// A redirect; `None` when `Location` is missing or unusable.
    Redirect(Option<Url>),
    Final {
        body: Vec<u8>,
        retry_after: Option<Duration>,
    },
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

async fn read_capped(mut response: Response) -> Result<Vec<u8>, FetchError> {
    let mut body = Vec::new();
    while body.len() < MAX_BODY {
        let Some(chunk) = response.chunk().await.map_err(map_err)? else {
            break;
        };
        let room = MAX_BODY - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }
    Ok(body)
}

fn map_err(err: reqwest::Error) -> FetchError {
    if err.is_timeout() {
        return FetchError::Timeout;
    }
    // Top-level text embeds the URL, which must not steer classification.
    let err = err.without_url();
    let mut messages = Vec::new();
    let mut source: Option<&(dyn Error + 'static)> = Some(&err);
    while let Some(e) = source {
        messages.push(e.to_string().to_lowercase());
        source = e.source();
    }
    let mentions = |needles: &[&str]| {
        messages
            .iter()
            .any(|message| needles.iter().any(|needle| message.contains(needle)))
    };
    if mentions(&[
        "dns error",
        "failed to lookup address",
        "name or service not known",
        "nodename nor servname",
        "no such host",
    ]) {
        FetchError::Dns
    } else if mentions(&["certificate", "tls", "ssl", "handshake"]) {
        FetchError::Tls
    } else {
        FetchError::Connect
    }
}

/// What a fetch says about a URL, before soft-404 checks and retries.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Verdict {
    Ok {
        chain: Vec<Hop>,
    },
    /// Moved for good; `fix` is the target of the last 301/308 in the leading run.
    Permanent {
        chain: Vec<Hop>,
        fix: Url,
    },
    Temporary {
        chain: Vec<Hop>,
        final_url: Url,
    },
    Dead {
        reason: String,
    },
    Suspect {
        chain: Vec<Hop>,
        reason: String,
    },
    Unverifiable {
        reason: String,
    },
}

/// Classifies a fetch outcome by its status and redirect chain.
#[must_use]
pub fn classify(outcome: &Result<Fetched, FetchError>) -> Verdict {
    let unverifiable = |reason: String| Verdict::Unverifiable { reason };
    let fetched = match outcome {
        Ok(fetched) => fetched,
        Err(FetchError::Dns) => {
            return Verdict::Dead {
                reason: FetchError::Dns.to_string(),
            };
        }
        Err(err) => return unverifiable(err.to_string()),
    };
    let chain = &fetched.chain;
    let Some(last) = chain.last() else {
        return unverifiable("no response".to_owned());
    };
    match last.status {
        404 | 410 => Verdict::Dead {
            reason: last.status.to_string(),
        },
        401 | 403 | 429 | 999 | 500..=599 => unverifiable(last.status.to_string()),
        200..=299 => {
            let leading = chain
                .iter()
                .take_while(|hop| matches!(hop.status, 301 | 308))
                .count();
            if leading > 0 {
                // The run ends in a 2xx response, so a next hop exists.
                let fix = chain[leading].url.clone();
                Verdict::Permanent {
                    chain: chain.clone(),
                    fix,
                }
            } else if matches!(chain[0].status, 302 | 303 | 307) {
                Verdict::Temporary {
                    chain: chain.clone(),
                    final_url: last.url.clone(),
                }
            } else if chain.len() == 1 {
                Verdict::Ok {
                    chain: chain.clone(),
                }
            } else {
                unverifiable(format!("unexpected redirect {}", chain[0].status))
            }
        }
        status => unverifiable(status.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn fetcher(extra: &str) -> Fetcher {
        let text = format!(
            "site_url = \"https://site.example/\"\nuser_agent = \"rotguard-unit/1.0\"\n{extra}"
        );
        Fetcher::new(&config::parse(&text).expect("test config is valid"))
    }

    fn url(server: &MockServer, p: &str) -> Url {
        Url::parse(&format!("{}{p}", server.uri())).expect("mock uri is valid")
    }

    fn redirect(status: u16, to: &str) -> ResponseTemplate {
        ResponseTemplate::new(status).insert_header("Location", to)
    }

    #[tokio::test]
    async fn records_a_redirect_chain_hop_by_hop() {
        let server = MockServer::start().await;
        Mock::given(path("/a"))
            .respond_with(redirect(301, "/b"))
            .mount(&server)
            .await;
        Mock::given(path("/b"))
            .respond_with(redirect(302, "/c"))
            .mount(&server)
            .await;
        Mock::given(path("/c"))
            .respond_with(ResponseTemplate::new(200).set_body_string("hi"))
            .mount(&server)
            .await;
        let got = fetcher("host_interval_ms = 0")
            .fetch(&url(&server, "/a"))
            .await
            .unwrap();
        let hops: Vec<_> = got.chain.iter().map(|h| (h.url.path(), h.status)).collect();
        assert_eq!(hops, [("/a", 301), ("/b", 302), ("/c", 200)]);
        assert_eq!(got.body, b"hi");
    }

    #[tokio::test]
    async fn self_redirect_is_a_loop() {
        let server = MockServer::start().await;
        Mock::given(path("/x"))
            .respond_with(redirect(302, "/x"))
            .mount(&server)
            .await;
        let err = fetcher("host_interval_ms = 0")
            .fetch(&url(&server, "/x"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, FetchError::Loop(ref c) if c.len() == 1),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn redirect_without_location_is_reported() {
        let server = MockServer::start().await;
        Mock::given(path("/x"))
            .respond_with(ResponseTemplate::new(302))
            .mount(&server)
            .await;
        let err = fetcher("host_interval_ms = 0")
            .fetch(&url(&server, "/x"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, FetchError::NoLocation(ref c) if c.len() == 1),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn eleven_redirects_are_too_many() {
        let server = MockServer::start().await;
        for i in 0..11 {
            Mock::given(path(format!("/h{i}")))
                .respond_with(redirect(302, &format!("/h{}", i + 1)))
                .mount(&server)
                .await;
        }
        Mock::given(path("/h11"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let err = fetcher("host_interval_ms = 0")
            .fetch(&url(&server, "/h0"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, FetchError::TooManyHops(ref c) if c.len() == 11),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn ten_redirects_still_succeed() {
        let server = MockServer::start().await;
        for i in 0..10 {
            Mock::given(path(format!("/h{i}")))
                .respond_with(redirect(302, &format!("/h{}", i + 1)))
                .mount(&server)
                .await;
        }
        Mock::given(path("/h10"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let got = fetcher("host_interval_ms = 0")
            .fetch(&url(&server, "/h0"))
            .await
            .unwrap();
        assert_eq!(got.chain.len(), 11);
    }

    #[tokio::test]
    async fn body_is_cut_at_256_kb() {
        let server = MockServer::start().await;
        Mock::given(path("/big"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Retry-After", "7")
                    .set_body_bytes(vec![b'x'; 300 * 1024]),
            )
            .mount(&server)
            .await;
        let got = fetcher("host_interval_ms = 0")
            .fetch(&url(&server, "/big"))
            .await
            .unwrap();
        assert_eq!(got.body.len(), 256 * 1024);
        assert_eq!(got.retry_after, Some(Duration::from_secs(7)));
    }

    struct Stamp(Arc<StdMutex<Vec<Instant>>>);

    impl Respond for Stamp {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            self.0.lock().unwrap().push(Instant::now());
            ResponseTemplate::new(200)
        }
    }

    #[tokio::test]
    async fn requests_to_one_host_are_spaced_and_carry_the_user_agent() {
        let server = MockServer::start().await;
        let starts = Arc::new(StdMutex::new(Vec::new()));
        Mock::given(method("GET"))
            .and(header("user-agent", "rotguard-unit/1.0"))
            .respond_with(Stamp(Arc::clone(&starts)))
            .expect(3)
            .mount(&server)
            .await;
        let f = fetcher("host_interval_ms = 300");
        let (u1, u2, u3) = (url(&server, "/1"), url(&server, "/2"), url(&server, "/3"));
        let (a, b, c) = tokio::join!(f.fetch(&u1), f.fetch(&u2), f.fetch(&u3));
        for r in [a, b, c] {
            assert_eq!(r.unwrap().chain[0].status, 200);
        }
        let mut starts = starts.lock().unwrap().clone();
        starts.sort();
        assert_eq!(starts.len(), 3);
        for pair in starts.windows(2) {
            assert!(
                pair[1] - pair[0] >= Duration::from_millis(260),
                "{:?}",
                pair[1] - pair[0]
            );
        }
        let total = starts[2] - starts[0];
        assert!(total >= Duration::from_millis(560), "{total:?}");
    }

    struct Slow;

    impl Respond for Slow {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            ResponseTemplate::new(200).set_delay(Duration::from_millis(200))
        }
    }

    #[tokio::test]
    async fn one_request_in_flight_per_host() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(Slow)
            .mount(&server)
            .await;
        let f = fetcher("host_interval_ms = 0");
        let (u1, u2, u3) = (url(&server, "/1"), url(&server, "/2"), url(&server, "/3"));
        let begun = Instant::now();
        let (a, b, c) = tokio::join!(f.fetch(&u1), f.fetch(&u2), f.fetch(&u3));
        for r in [a, b, c] {
            r.unwrap();
        }
        assert!(
            begun.elapsed() >= Duration::from_millis(580),
            "{:?}",
            begun.elapsed()
        );
    }

    #[tokio::test]
    async fn non_ascii_location_is_followed() {
        let server = MockServer::start().await;
        Mock::given(path("/a"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("Location", "/caf\u{e9}".as_bytes()),
            )
            .mount(&server)
            .await;
        Mock::given(path("/caf%C3%A9"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let got = fetcher("host_interval_ms = 0")
            .fetch(&url(&server, "/a"))
            .await
            .unwrap();
        assert_eq!(got.chain.len(), 2);
        assert_eq!(got.chain[1].status, 200);
    }

    #[tokio::test]
    async fn redirect_to_a_non_http_url_is_no_location() {
        let server = MockServer::start().await;
        Mock::given(path("/a"))
            .respond_with(redirect(302, "mailto:someone"))
            .mount(&server)
            .await;
        let err = fetcher("host_interval_ms = 0")
            .fetch(&url(&server, "/a"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, FetchError::NoLocation(ref c) if c.len() == 1),
            "{err:?}"
        );
    }

    #[test]
    fn errors_display_their_reason() {
        let shown: Vec<_> = [
            FetchError::Dns,
            FetchError::Timeout,
            FetchError::Tls,
            FetchError::Connect,
            FetchError::NoLocation(Vec::new()),
            FetchError::Loop(Vec::new()),
            FetchError::TooManyHops(Vec::new()),
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(
            shown,
            [
                "dns",
                "timeout",
                "tls",
                "connection",
                "no location",
                "redirect loop",
                "too many hops"
            ]
        );
    }

    #[tokio::test]
    async fn a_slow_handler_times_out() {
        let server = MockServer::start().await;
        Mock::given(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
            .mount(&server)
            .await;
        let err = fetcher("host_interval_ms = 0\ntimeout_ms = 200")
            .fetch(&url(&server, "/slow"))
            .await
            .unwrap_err();
        assert_eq!(err, FetchError::Timeout);
    }

    #[tokio::test]
    async fn a_refused_connection_is_connect() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        drop(listener);
        let err = fetcher("host_interval_ms = 0")
            .fetch(&dead)
            .await
            .unwrap_err();
        assert_eq!(err, FetchError::Connect);
    }

    #[tokio::test]
    async fn a_tls_looking_url_does_not_make_a_refusal_tls() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = Url::parse(&format!(
            "http://{}/ssl-tls-certificate-handshake",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        drop(listener);
        let err = fetcher("host_interval_ms = 0")
            .fetch(&dead)
            .await
            .unwrap_err();
        assert_eq!(err, FetchError::Connect);
    }

    fn hop(status: u16, p: &str) -> Hop {
        Hop {
            url: Url::parse(&format!("https://m.example/{p}")).expect("literal url"),
            status,
        }
    }

    fn fetched(hops: &[(u16, &str)]) -> Result<Fetched, FetchError> {
        Ok(Fetched {
            chain: hops.iter().map(|&(s, p)| hop(s, p)).collect(),
            body: Vec::new(),
            retry_after: None,
        })
    }

    fn unverifiable(reason: &str) -> Verdict {
        Verdict::Unverifiable {
            reason: reason.to_owned(),
        }
    }

    fn dead(reason: &str) -> Verdict {
        Verdict::Dead {
            reason: reason.to_owned(),
        }
    }

    #[test]
    fn classifies_outcomes() {
        let ok = fetched(&[(200, "a")]);
        let permanent = fetched(&[(301, "p1"), (200, "p2")]);
        let permanent_run = fetched(&[(308, "p1"), (302, "p2"), (200, "p3")]);
        let permanent_two = fetched(&[(301, "p1"), (308, "p2"), (200, "p3")]);
        let temporary = fetched(&[(307, "t1"), (301, "t2"), (200, "t3")]);
        let cases: Vec<(&str, Result<Fetched, FetchError>, Verdict)> = vec![
            ("404", fetched(&[(404, "a")]), dead("404")),
            ("410", fetched(&[(410, "a")]), dead("410")),
            (
                "301 then 404",
                fetched(&[(301, "a"), (404, "b")]),
                dead("404"),
            ),
            ("dns", Err(FetchError::Dns), dead("dns")),
            ("401", fetched(&[(401, "a")]), unverifiable("401")),
            ("403", fetched(&[(403, "a")]), unverifiable("403")),
            ("429", fetched(&[(429, "a")]), unverifiable("429")),
            ("999", fetched(&[(999, "a")]), unverifiable("999")),
            ("503", fetched(&[(503, "a")]), unverifiable("503")),
            ("timeout", Err(FetchError::Timeout), unverifiable("timeout")),
            ("tls", Err(FetchError::Tls), unverifiable("tls")),
            (
                "connect",
                Err(FetchError::Connect),
                unverifiable("connection"),
            ),
            (
                "permanent",
                permanent.clone(),
                Verdict::Permanent {
                    chain: permanent.unwrap().chain,
                    fix: hop(200, "p2").url,
                },
            ),
            (
                "308 then 302 fixes to the end of the leading run",
                permanent_run.clone(),
                Verdict::Permanent {
                    chain: permanent_run.unwrap().chain,
                    fix: hop(302, "p2").url,
                },
            ),
            (
                "301 then 308 fixes to the last target",
                permanent_two.clone(),
                Verdict::Permanent {
                    chain: permanent_two.unwrap().chain,
                    fix: hop(200, "p3").url,
                },
            ),
            (
                "temporary ignores later 301",
                temporary.clone(),
                Verdict::Temporary {
                    chain: temporary.unwrap().chain,
                    final_url: hop(200, "t3").url,
                },
            ),
            (
                "ok",
                ok.clone(),
                Verdict::Ok {
                    chain: ok.unwrap().chain,
                },
            ),
            ("405", fetched(&[(405, "a")]), unverifiable("405")),
            (
                "no location",
                Err(FetchError::NoLocation(vec![hop(301, "a")])),
                unverifiable("no location"),
            ),
            (
                "loop",
                Err(FetchError::Loop(vec![hop(301, "a")])),
                unverifiable("redirect loop"),
            ),
            (
                "too many hops",
                Err(FetchError::TooManyHops(vec![hop(301, "a")])),
                unverifiable("too many hops"),
            ),
        ];
        for (name, outcome, expected) in cases {
            assert_eq!(classify(&outcome), expected, "{name}");
        }
    }
}
