//! Fetching URLs politely (one request in flight per host, spaced starts, bounded hosts),
//! classifying the outcome, and checking many URLs with retries under a deadline.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::sync::{Arc, LazyLock, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex::Regex;
use reqwest::header::{LOCATION, RETRY_AFTER};
use reqwest::redirect::Policy;
use reqwest::{Client, Response, StatusCode};
use serde::Serialize;
use tokio::sync::{Mutex, OnceCell, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep, sleep_until, timeout_at};
use url::Url;

use crate::config::{Config, HostConfig};

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
    /// A lookup that failed without proving the host is gone.
    DnsFailed,
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
            Self::DnsFailed => "dns lookup failed",
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

/// The checker's client: redirects off (the fetcher follows them by hand), the config's
/// timeout and User-Agent.
///
/// # Errors
/// If the TLS backend cannot load the system's CA certificates.
pub fn http_client(cfg: &Config) -> Result<Client, reqwest::Error> {
    Client::builder()
        .redirect(Policy::none())
        .timeout(cfg.timeout)
        .user_agent(cfg.user_agent.as_str())
        .build()
}

/// Fetches URLs through per-host gates.
#[derive(Debug)]
pub struct Fetcher {
    client: Client,
    interval: Duration,
    gates: StdMutex<HashMap<String, Arc<Mutex<Option<Instant>>>>>,
    hosts: Semaphore,
}

impl Fetcher {
    /// A fetcher that sends its requests through `client`, built by [`http_client`].
    #[must_use]
    pub fn new(client: Client, cfg: &Config) -> Fetcher {
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
        let body = match read_capped(response).await {
            Ok(body) => body,
            // The status is the answer; only 2xx bodies feed soft-404 checks.
            Err(_) if !status.is_success() => Vec::new(),
            Err(err) => return Err(err),
        };
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
    error_kind(&messages)
}

/// Classifies lowercased messages: the error without its URL, then each source.
pub fn error_kind(messages: &[String]) -> FetchError {
    let mentions = |needles: &[&str]| {
        messages
            .iter()
            .any(|message| needles.iter().any(|needle| message.contains(needle)))
    };
    if mentions(&[
        "name or service not known",
        "nodename nor servname provided",
        "no such host",
        "no address associated with hostname",
    ]) {
        FetchError::Dns
    } else if mentions(&[
        "dns error",
        "failed to lookup address",
        "temporary failure in name resolution",
    ]) {
        FetchError::DnsFailed
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
    /// The host answers a made-up sibling URL with the same 401, 403 or 999.
    Blocked {
        status: u16,
    },
}

/// Whether a non-root URL's chain has a hop after the first at the root, which a host with
/// `soft404` treats as a removed page. A chain ending in 2xx is judged by `classify` instead.
fn redirects_to_root(chain: &[Hop], host: &HostConfig) -> bool {
    host.soft404
        && chain.split_first().is_some_and(|(first, rest)| {
            first.url.path() != "/" && rest.iter().any(|hop| hop.url.path() == "/")
        })
}

/// Classifies a fetch outcome by its status and redirect chain.
///
/// A non-root URL is a soft-404 suspect, unless `host` turns soft-404 detection off, when its
/// 2xx chain ends at the site root or its leading 301/308 run does; any other chain is suspect
/// when it fails or ends in an unexplained status after passing through the root. A middle root
/// hop in a chain that ends elsewhere doesn't count.
#[must_use]
pub fn classify(outcome: &Result<Fetched, FetchError>, host: &HostConfig) -> Verdict {
    let unverifiable = |reason: String| Verdict::Unverifiable { reason };
    let fetched = match outcome {
        Ok(fetched) => fetched,
        Err(FetchError::Dns) => {
            return Verdict::Dead {
                reason: FetchError::Dns.to_string(),
            };
        }
        Err(
            FetchError::NoLocation(chain)
            | FetchError::Loop(chain)
            | FetchError::TooManyHops(chain),
        ) if redirects_to_root(chain, host) => {
            return Verdict::Suspect {
                chain: chain.clone(),
                reason: "redirect to root".to_owned(),
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
        status if !(200..=299).contains(&status) && redirects_to_root(chain, host) => {
            Verdict::Suspect {
                chain: chain.clone(),
                reason: "redirect to root".to_owned(),
            }
        }
        200..=299 => {
            let leading = chain
                .iter()
                .take_while(|hop| matches!(hop.status, 301 | 308))
                .count();
            let is_root = |hop: &Hop| hop.url.path() == "/";
            if host.soft404
                && !is_root(&chain[0])
                && (is_root(last) || (leading > 0 && is_root(&chain[leading])))
            {
                Verdict::Suspect {
                    chain: chain.clone(),
                    reason: "redirect to root".to_owned(),
                }
            } else if leading > 0 {
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

/// Most retries after a timeout, a 5xx or a connection error.
const TRANSIENT_RETRIES: u32 = 2;
/// Longest wait honoured for a 429's `Retry-After`.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

/// Checks every URL concurrently, within the fetcher's gates.
///
/// Retries transient failures, re-checks a first `Dead` after `cfg.recheck_delay`, and gives
/// every URL still unfinished at `deadline` the verdict `Unverifiable("deadline")`.
/// Calls `on_done(n)` after each URL finishes, `n` being the count done so far; a URL cut off at
/// the deadline counts.
pub async fn check_all(
    urls: Vec<Url>,
    client: Client,
    cfg: &Config,
    deadline: Instant,
    mut on_done: impl FnMut(usize),
) -> HashMap<Url, Verdict> {
    let fetcher = Arc::new(Fetcher::new(client, cfg));
    let probes = Arc::new(Probes::default());
    let recheck_delay = cfg.recheck_delay;
    let phrases: Arc<[String]> = cfg.not_found_phrases.as_slice().into();
    let mut tasks = JoinSet::new();
    for url in urls {
        let fetcher = Arc::clone(&fetcher);
        let probes = Arc::clone(&probes);
        let phrases = Arc::clone(&phrases);
        let host = url
            .host_str()
            .and_then(|name| cfg.hosts.get(name))
            .cloned()
            .unwrap_or_default();
        tasks.spawn(async move {
            let check = check_one(&fetcher, &probes, &phrases, &url, &host, recheck_delay);
            let verdict =
                timeout_at(deadline, check)
                    .await
                    .unwrap_or_else(|_| Verdict::Unverifiable {
                        reason: "deadline".to_owned(),
                    });
            (url, verdict)
        });
    }
    let mut verdicts = HashMap::with_capacity(tasks.len());
    while let Some(joined) = tasks.join_next().await {
        // A task only fails by panicking, which is a bug to surface, not to hide.
        let (url, verdict) = joined.expect("a check task does not panic");
        verdicts.insert(url, verdict);
        on_done(verdicts.len());
    }
    verdicts
}

/// One URL: fetch with retries; a first `Dead` is fetched again after `recheck_delay`; a 2xx
/// that passes classification is probed for soft-404 behaviour and its title is checked
/// against the not-found `phrases`.
async fn check_one(
    fetcher: &Fetcher,
    probes: &Probes,
    phrases: &[String],
    url: &Url,
    host: &HostConfig,
    recheck_delay: Duration,
) -> Verdict {
    let mut outcome = fetch_with_retries(fetcher, url).await;
    if matches!(classify(&outcome, host), Verdict::Dead { .. }) {
        sleep(recheck_delay).await;
        outcome = fetch_with_retries(fetcher, url).await;
    }
    let verdict = classify(&outcome, host);
    if let Ok(fetched) = &outcome
        && matches!(verdict, Verdict::Unverifiable { .. })
        && let Some(last) = fetched.chain.last()
        && matches!(last.status, 401 | 403 | 999)
    {
        let probe = probes.probe(fetcher, &last.url).await;
        return match *probe {
            ProbeResult::Answered { status, .. } if status == last.status => {
                Verdict::Blocked { status }
            }
            _ => verdict,
        };
    }
    let (Ok(fetched), true) = (
        &outcome,
        host.soft404
            && matches!(
                verdict,
                Verdict::Ok { .. } | Verdict::Permanent { .. } | Verdict::Temporary { .. }
            ),
    ) else {
        return verdict;
    };
    match probes.reason(fetcher, url, fetched, phrases).await {
        Soft404::Suspect(reason) => Verdict::Suspect {
            chain: fetched.chain.clone(),
            reason,
        },
        Soft404::CantTell(reason) => Verdict::Unverifiable { reason },
        Soft404::NoEvidence => verdict,
    }
}

/// Length of a probe's random last path segment.
const PROBE_TOKEN_LEN: usize = 24;
/// Shingle similarity from which two bodies count as the same page.
const SIMILAR_BODY: f64 = 0.9;
/// Shingle similarity below which two bodies count as different pages.
const DIFFERENT_BODY: f64 = 0.5;
/// Fewest shingles each body needs for a low similarity to count as evidence.
const MIN_SHINGLES: usize = 20;

/// What a probe URL returned, kept for comparing with every page under the same parent path.
#[derive(Debug)]
enum ProbeResult {
    /// The probe ended in a response with this status.
    Answered {
        status: u16,
        final_url: Url,
        title: String,
        shingles: HashSet<String>,
    },
    /// The probe could not be fetched.
    Failed,
}

/// What the soft-404 checks found out about a 2xx link.
#[derive(Debug, PartialEq, Eq)]
enum Soft404 {
    /// Nothing says the page is a soft 404.
    NoEvidence,
    /// The page looks like a soft 404, for this reason.
    Suspect(String),
    /// The page matches the probe, but the match says nothing about the link: the page needs
    /// JavaScript, or the site's root page matches the probe too.
    CantTell(String),
}

/// `Suspect` when the page `title` contains one of the not-found `phrases`.
fn title_says_not_found(title: &str, phrases: &[String]) -> Soft404 {
    let lowered = title.to_lowercase();
    phrases
        .iter()
        .find_map(|phrase| {
            let phrase = phrase.split_whitespace().collect::<Vec<_>>().join(" ");
            let phrase = phrase.to_lowercase();
            (!phrase.is_empty() && lowered.contains(&phrase))
                .then(|| format!("title says \"{phrase}\""))
        })
        .map_or(Soft404::NoEvidence, Soft404::Suspect)
}

/// Phrases (lowercase, single-spaced) that say a page's content is built by JavaScript.
const NEEDS_JAVASCRIPT: [&str; 4] = [
    "enable javascript",
    "javascript is required",
    "requires javascript",
    "javascript is disabled",
];

/// A probe match that says nothing about the link: a not-found title still makes it a
/// `Suspect`, anything else is `CantTell(reason)`.
fn suspect_or_cant_tell(title: &str, phrases: &[String], reason: &str) -> Soft404 {
    match title_says_not_found(title, phrases) {
        found @ Soft404::Suspect(_) => found,
        Soft404::NoEvidence | Soft404::CantTell(_) => Soft404::CantTell(reason.to_owned()),
    }
}

/// Whether the page's visible text contains a needs-JavaScript phrase.
fn needs_javascript(html: &str) -> bool {
    let visible = visible_text(html);
    NEEDS_JAVASCRIPT
        .iter()
        .any(|phrase| visible.contains(phrase))
}

/// Probes by (origin, parent path, whether the path ends in `/`); each cell is filled by the
/// first check that needs it.
type ProbeCache = HashMap<(String, String, bool), Arc<OnceCell<Arc<ProbeResult>>>>;

/// What a site's root page returned, the control for a probe match.
#[derive(Debug)]
enum RootPage {
    /// The root ended in 2xx.
    Usable {
        title: String,
        shingles: HashSet<String>,
    },
    /// The root could not be fetched or did not end in 2xx.
    Unusable,
}

/// Probe results shared by all checks, one fetch per (origin, parent path, whether the path
/// ends in `/`), and one root page per origin.
#[derive(Debug, Default)]
struct Probes {
    cache: StdMutex<ProbeCache>,
    roots: StdMutex<HashMap<String, Arc<OnceCell<Arc<RootPage>>>>>,
}

impl Probes {
    /// Whether `url`, which ended in 2xx `fetched`, looks like a soft 404, by the probe or the
    /// title. A non-root URL is probed first; the root has no probe.
    async fn reason(
        &self,
        fetcher: &Fetcher,
        url: &Url,
        fetched: &Fetched,
        phrases: &[String],
    ) -> Soft404 {
        let Some(last) = fetched.chain.last() else {
            return Soft404::NoEvidence;
        };
        let text = String::from_utf8_lossy(&fetched.body);
        let title = title_of(&text);
        let mut probe_status = None;
        if url.path() != "/" {
            let probe = self.probe(fetcher, url).await;
            if let ProbeResult::Answered {
                status,
                final_url,
                title: probe_title,
                shingles: probe_shingles,
            } = &*probe
            {
                probe_status = Some(*status);
                // Only a 2xx probe is a page to compare with.
                if (200..300).contains(status) {
                    if last.url == *final_url {
                        // A probe redirected to a link that was not itself redirected showed
                        // the link's own page, which says nothing about a soft 404.
                        if fetched.chain.len() > 1 {
                            return Soft404::Suspect("probe: same final URL".to_owned());
                        }
                        return title_says_not_found(&title, phrases);
                    }
                    let link_shingles = shingles(&text);
                    let similarity = jaccard(&link_shingles, probe_shingles);
                    // A shared title over clearly different text is a site-wide title.
                    let different = similarity < DIFFERENT_BODY
                        && link_shingles.len() >= MIN_SHINGLES
                        && probe_shingles.len() >= MIN_SHINGLES;
                    let title_matched = !title.is_empty() && title == *probe_title && !different;
                    let body_matched = similarity >= SIMILAR_BODY;
                    if title_matched || body_matched {
                        if needs_javascript(&text) {
                            return suspect_or_cant_tell(
                                &title,
                                phrases,
                                "probe: page needs JavaScript",
                            );
                        }
                        // The root answering like the probe on every matched signal means the
                        // site serves one page for every URL.
                        if let RootPage::Usable {
                            title: root_title,
                            shingles: root_shingles,
                        } = &*self.root(fetcher, url).await
                            && (!title_matched || root_title == probe_title)
                            && (!body_matched
                                || jaccard(root_shingles, probe_shingles) >= DIFFERENT_BODY)
                        {
                            return suspect_or_cant_tell(
                                &title,
                                phrases,
                                "probe: every URL gets the same page",
                            );
                        }
                        let reason = if title_matched {
                            "probe: same title"
                        } else {
                            "probe: similar body"
                        };
                        return Soft404::Suspect(reason.to_owned());
                    }
                }
            }
        }
        if matches!(probe_status, Some(404 | 410)) {
            return Soft404::NoEvidence;
        }
        title_says_not_found(&title, phrases)
    }

    /// The root page of `url`'s origin, fetched by the first caller.
    async fn root(&self, fetcher: &Fetcher, url: &Url) -> Arc<RootPage> {
        let cell = {
            let mut roots = self.roots.lock().expect("no code panics holding this lock");
            Arc::clone(roots.entry(url.origin().ascii_serialization()).or_default())
        };
        let mut target = url.clone();
        target.set_path("/");
        target.set_query(None);
        target.set_fragment(None);
        let root = cell
            .get_or_init(|| async {
                let Ok(fetched) = fetcher.fetch(&target).await else {
                    return Arc::new(RootPage::Unusable);
                };
                match fetched.chain.last() {
                    Some(last) if (200..300).contains(&last.status) => {
                        let text = String::from_utf8_lossy(&fetched.body);
                        Arc::new(RootPage::Usable {
                            title: title_of(&text),
                            shingles: shingles(&text),
                        })
                    }
                    _ => Arc::new(RootPage::Unusable),
                }
            })
            .await;
        Arc::clone(root)
    }

    /// The probe for `url`'s parent path, fetched by the first caller. A path ending in `/`
    /// (other than `/`) is a directory: its last segment is replaced and the `/` kept.
    async fn probe(&self, fetcher: &Fetcher, url: &Url) -> Arc<ProbeResult> {
        let path = url.path();
        let dir = path.len() > 1 && path.ends_with('/');
        let path = if dir { &path[..path.len() - 1] } else { path };
        let parent = path.rsplit_once('/').map_or("/", |(parent, _)| parent);
        let cell = {
            let mut cache = self.cache.lock().expect("no code panics holding this lock");
            let key = (url.origin().ascii_serialization(), parent.to_owned(), dir);
            Arc::clone(cache.entry(key).or_default())
        };
        let mut target = url.clone();
        let slash = if dir { "/" } else { "" };
        target.set_path(&format!("{parent}/{}{slash}", random_token()));
        target.set_query(None);
        target.set_fragment(None);
        let probe = cell
            .get_or_init(|| async {
                let Ok(probed) = fetcher.fetch(&target).await else {
                    return Arc::new(ProbeResult::Failed);
                };
                let Some(last) = probed.chain.last() else {
                    return Arc::new(ProbeResult::Failed);
                };
                let text = String::from_utf8_lossy(&probed.body);
                Arc::new(ProbeResult::Answered {
                    status: last.status,
                    final_url: last.url.clone(),
                    title: title_of(&text),
                    shingles: shingles(&text),
                })
            })
            .await;
        Arc::clone(probe)
    }
}

/// A random `[a-z0-9]` token built from std's randomly keyed hasher.
fn random_token() -> String {
    const ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    (0..PROBE_TOKEN_LEN)
        .map(|i| {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_usize(i);
            hasher.write_u128(nanos);
            let index = usize::try_from(hasher.finish() % 36).expect("below 36");
            char::from(ALPHABET[index])
        })
        .collect()
}

static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("regex is a literal"));
static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").expect("regex is a literal"));

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// The page's `<title>`, trimmed with inner whitespace collapsed; empty when it has none.
fn title_of(html: &str) -> String {
    TITLE.captures(html).map_or_else(String::new, |caps| {
        decode_entities(&caps[1])
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// The openers of hidden blocks and the closer that ends each.
const HIDDEN_BLOCKS: [(&str, &str); 3] = [
    ("<!--", "-->"),
    ("<script", "</script>"),
    ("<style", "</style>"),
];

/// Replaces comments, scripts and styles with a space in one left-to-right scan. An opener
/// with no closer hides the rest of the body.
fn strip_hidden(html: &str) -> String {
    // ASCII lowercasing keeps byte offsets, so `lower` indexes `html` too.
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;
    // Each opener's next position, searched again only once the scan has passed it, so no
    // stretch of the body is searched twice for the same opener.
    let mut next: [Option<usize>; 3] = [None; 3];
    let mut searched = [0usize; 3];
    loop {
        for (i, &(opener, _)) in HIDDEN_BLOCKS.iter().enumerate() {
            if searched[i] <= pos {
                (next[i], searched[i]) = find_opener(&lower, pos, opener);
            }
        }
        let Some((i, start)) = next
            .iter()
            .enumerate()
            .filter_map(|(i, at)| at.map(|at| (i, at)))
            .min_by_key(|&(_, at)| at)
        else {
            break;
        };
        let (opener, closer) = HIDDEN_BLOCKS[i];
        out.push_str(&html[pos..start]);
        out.push(' ');
        let body = start + opener.len();
        let Some(len) = lower[body..].find(closer) else {
            return out;
        };
        pos = body + len + closer.len();
    }
    out.push_str(&html[pos..]);
    out
}

/// The first `opener` at or after `from` that counts (`<script` and `<style` must be followed
/// by a non-word character, like a regex `\b`), and how far the search has covered.
fn find_opener(lower: &str, from: usize, opener: &str) -> (Option<usize>, usize) {
    let mut at = from;
    while let Some(found) = lower[at..].find(opener) {
        let start = at + found;
        let after = lower[start + opener.len()..].chars().next();
        if opener == "<!--" || after.is_none_or(|c| !(c.is_alphanumeric() || c == '_')) {
            return (Some(start), start);
        }
        at = start + opener.len();
    }
    (None, lower.len() + 1)
}

/// The page's visible text: no comments, scripts, styles or tags, entities decoded, lowercased,
/// whitespace collapsed to single spaces.
fn visible_text(html: &str) -> String {
    let visible = strip_hidden(html);
    let visible = TAG.replace_all(&visible, " ");
    decode_entities(&visible)
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The set of 3-word shingles of the page's visible, lowercased text.
fn shingles(html: &str) -> HashSet<String> {
    let visible = visible_text(html);
    let words: Vec<&str> = visible
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    words.windows(3).map(|w| w.join(" ")).collect()
}

/// Jaccard similarity of two sets; 0 when both are empty, so empty pages never match.
fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    // Set sizes are far below f64's exact integer range.
    #[expect(clippy::cast_precision_loss, reason = "counts are far below 2^52")]
    let ratio = a.intersection(b).count() as f64 / union as f64;
    ratio
}

/// What to do after an outcome.
enum Retry {
    No,
    Transient,
    RateLimited(Option<Duration>),
}

fn retry_for(outcome: &Result<Fetched, FetchError>) -> Retry {
    match outcome {
        Err(FetchError::Timeout | FetchError::Connect | FetchError::DnsFailed) => Retry::Transient,
        Err(_) => Retry::No,
        Ok(fetched) => match fetched.chain.last().map(|hop| hop.status) {
            Some(500..=599) => Retry::Transient,
            Some(429) => Retry::RateLimited(fetched.retry_after),
            _ => Retry::No,
        },
    }
}

async fn fetch_with_retries(fetcher: &Fetcher, url: &Url) -> Result<Fetched, FetchError> {
    let mut transient = 0;
    let mut rate_limited = false;
    loop {
        let outcome = fetcher.fetch(url).await;
        match retry_for(&outcome) {
            Retry::Transient if transient < TRANSIENT_RETRIES => transient += 1,
            Retry::RateLimited(after) if !rate_limited => {
                rate_limited = true;
                sleep(after.unwrap_or(fetcher.interval).min(MAX_RETRY_AFTER)).await;
            }
            Retry::No | Retry::Transient | Retry::RateLimited(_) => return outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use wiremock::matchers::{header, method, path, path_regex};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn fetcher(extra: &str) -> Fetcher {
        let text = format!(
            "site_url = \"https://site.example/\"\nuser_agent = \"rotguard-unit/1.0\"\n{extra}"
        );
        let cfg = config::parse(&text).expect("test config is valid");
        Fetcher::new(http_client(&cfg).expect("the test client builds"), &cfg)
    }

    fn url(server: &MockServer, p: &str) -> Url {
        Url::parse(&format!("{}{p}", server.uri())).expect("mock uri is valid")
    }

    fn redirect(status: u16, to: &str) -> ResponseTemplate {
        ResponseTemplate::new(status).insert_header("Location", to)
    }

    /// Serves one response whose `Content-Length` promises more than is sent, then hangs up.
    async fn truncated_url(status_line: &str) -> Url {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback binds");
        let addr = listener.local_addr().expect("bound socket has an address");
        let head = format!(
            "HTTP/1.1 {status_line}\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort"
        );
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("the test connects once");
            let mut request = [0u8; 1024];
            // The request content is irrelevant; read it so closing doesn't reset the socket.
            let _ = socket.read(&mut request).await;
            let _ = socket.write_all(head.as_bytes()).await;
            // Dropping the socket cuts the body short.
        });
        Url::parse(&format!("http://{addr}/page")).expect("loopback url is valid")
    }

    #[tokio::test]
    async fn a_truncated_body_keeps_the_non_2xx_status() {
        let target = truncated_url("404 Not Found").await;
        let outcome = fetcher("host_interval_ms = 0").fetch(&target).await;
        let fetched = outcome
            .as_ref()
            .expect("the status survives the body error");
        assert_eq!(fetched.chain.last().map(|h| h.status), Some(404));
        assert!(fetched.body.is_empty());
        assert_eq!(classify(&outcome, &HostConfig::default()), dead("404"));
    }

    #[tokio::test]
    async fn a_truncated_2xx_body_is_still_an_error() {
        let target = truncated_url("200 OK").await;
        let err = fetcher("host_interval_ms = 0")
            .fetch(&target)
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::Connect), "{err:?}");
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

    /// Label, routes served, expected outcome kind, chain length and last status.
    type RedirectCase<'a> = (
        &'a str,
        Vec<(String, ResponseTemplate)>,
        &'a str,
        usize,
        Option<u16>,
    );

    #[tokio::test]
    async fn redirect_edge_cases_end_as_expected() {
        let chain_of_302s = |count: usize| -> Vec<(String, ResponseTemplate)> {
            let mut routes: Vec<_> = (0..count)
                .map(|i| (format!("/h{i}"), redirect(302, &format!("/h{}", i + 1))))
                .collect();
            routes.push((format!("/h{count}"), ResponseTemplate::new(200)));
            routes
        };
        // (label, routes served, expected (outcome, chain length, last status))
        let cases: Vec<RedirectCase> = vec![
            (
                "self redirect is a loop",
                vec![("/h0".to_owned(), redirect(302, "/h0"))],
                "loop",
                1,
                None,
            ),
            (
                "redirect without location",
                vec![("/h0".to_owned(), ResponseTemplate::new(302))],
                "no location",
                1,
                None,
            ),
            (
                "redirect to a non-http url",
                vec![("/h0".to_owned(), redirect(302, "mailto:someone"))],
                "no location",
                1,
                None,
            ),
            (
                "eleven redirects are too many",
                chain_of_302s(11),
                "too many",
                11,
                None,
            ),
            (
                "ten redirects still succeed",
                chain_of_302s(10),
                "ok",
                11,
                Some(200),
            ),
            (
                "non-ascii location is followed",
                vec![
                    (
                        "/h0".to_owned(),
                        ResponseTemplate::new(302)
                            .insert_header("Location", "/caf\u{e9}".as_bytes()),
                    ),
                    ("/caf%C3%A9".to_owned(), ResponseTemplate::new(200)),
                ],
                "ok",
                2,
                Some(200),
            ),
        ];
        for (name, routes, kind, hops, last) in cases {
            let server = MockServer::start().await;
            for (route, response) in routes {
                Mock::given(path(route))
                    .respond_with(response)
                    .mount(&server)
                    .await;
            }
            let outcome = fetcher("host_interval_ms = 0")
                .fetch(&url(&server, "/h0"))
                .await;
            let got = match &outcome {
                Ok(f) => ("ok", f.chain.len(), f.chain.last().map(|h| h.status)),
                Err(FetchError::Loop(c)) => ("loop", c.len(), None),
                Err(FetchError::NoLocation(c)) => ("no location", c.len(), None),
                Err(FetchError::TooManyHops(c)) => ("too many", c.len(), None),
                Err(other) => panic!("{name}: unexpected {other:?}"),
            };
            assert_eq!(got, (kind, hops, last), "{name}");
        }
    }

    #[test]
    fn errors_display_their_reason() {
        let shown: Vec<_> = [
            FetchError::Dns,
            FetchError::DnsFailed,
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
                "dns lookup failed",
                "timeout",
                "tls",
                "connection",
                "no location",
                "redirect loop",
                "too many hops"
            ]
        );
    }

    #[test]
    fn a_failed_dns_lookup_retries_like_a_connect_failure() {
        assert!(matches!(
            retry_for(&Err(FetchError::DnsFailed)),
            Retry::Transient
        ));
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

    #[test]
    fn needs_javascript_matches_visible_text_only() {
        let cases = [
            ("<p>You need to ENABLE   JavaScript to run this.</p>", true),
            ("<noscript>Please enable\nJavaScript.</noscript>", true),
            ("<p>JavaScript is required</p>", true),
            ("<p>This site requires JavaScript.</p>", true),
            ("<p>JavaScript is disabled in your browser</p>", true),
            (
                "<script>alert('enable javascript')</script><p>Hello</p>",
                false,
            ),
            ("<!-- enable javascript --><p>Hello</p>", false),
            ("<p>Nothing about scripts here</p>", false),
        ];
        for (html, expected) in cases {
            assert_eq!(needs_javascript(html), expected, "{html}");
        }
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
        let temporary_302 = fetched(&[(302, "u1"), (200, "u2")]);
        let temporary_303 = fetched(&[(303, "v1"), (200, "v2")]);
        let cases: Vec<(&str, Result<Fetched, FetchError>, Verdict)> = vec![
            ("404", fetched(&[(404, "a")]), dead("404")),
            ("410", fetched(&[(410, "a")]), dead("410")),
            (
                "301 then 404",
                fetched(&[(301, "a"), (404, "b")]),
                dead("404"),
            ),
            ("dns", Err(FetchError::Dns), dead("dns")),
            (
                "dns failed",
                Err(FetchError::DnsFailed),
                unverifiable("dns lookup failed"),
            ),
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
                "302 then 200 is temporary",
                temporary_302.clone(),
                Verdict::Temporary {
                    chain: temporary_302.unwrap().chain,
                    final_url: hop(200, "u2").url,
                },
            ),
            (
                "303 then 200 is temporary",
                temporary_303.clone(),
                Verdict::Temporary {
                    chain: temporary_303.unwrap().chain,
                    final_url: hop(200, "v2").url,
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
            assert_eq!(
                classify(&outcome, &HostConfig::default()),
                expected,
                "{name}"
            );
        }
    }

    fn root_hop(status: u16) -> Hop {
        Hop {
            url: Url::parse("https://site.example/").unwrap(),
            status,
        }
    }

    fn deep_hop(status: u16) -> Hop {
        Hop {
            url: Url::parse("https://site.example/deep/page").unwrap(),
            status,
        }
    }

    fn hop_at(path: &str, status: u16) -> Hop {
        Hop {
            url: Url::parse(&format!("https://site.example{path}")).unwrap(),
            status,
        }
    }

    fn chain_of(hops: Vec<Hop>) -> Result<Fetched, FetchError> {
        let mut fetched = fetched(&[(200, "x")]).unwrap();
        fetched.chain = hops;
        Ok(fetched)
    }

    /// What a redirect chain should classify as; `Suspect` and `Permanent` borrow the chain.
    enum Expect {
        Suspect,
        Permanent(Url),
        Is(Verdict),
    }

    type RootCase<'a> = (&'a str, Result<Fetched, FetchError>, bool, Expect);

    #[test]
    fn redirects_to_the_root_classify_by_soft404_and_final_status() {
        let loop_chain = || vec![deep_hop(301), root_hop(301)];
        let cases: Vec<RootCase> = vec![
            (
                "301 to the root",
                chain_of(vec![deep_hop(301), root_hop(200)]),
                true,
                Expect::Suspect,
            ),
            (
                "302 to the root is suspect, not temporary",
                chain_of(vec![deep_hop(302), root_hop(200)]),
                true,
                Expect::Suspect,
            ),
            (
                "permanent when soft404 is off",
                chain_of(vec![deep_hop(301), root_hop(200)]),
                false,
                Expect::Permanent(root_hop(200).url),
            ),
            (
                "a root url redirected to the root is unaffected",
                chain_of(vec![root_hop(301), root_hop(200)]),
                true,
                Expect::Permanent(root_hop(200).url),
            ),
            (
                "ending in 404 stays dead",
                chain_of(vec![deep_hop(301), root_hop(404)]),
                true,
                Expect::Is(dead("404")),
            ),
            (
                "301 -> 400",
                chain_of(vec![deep_hop(301), root_hop(400)]),
                true,
                Expect::Suspect,
            ),
            (
                "302 -> 405",
                chain_of(vec![deep_hop(302), root_hop(405)]),
                true,
                Expect::Suspect,
            ),
            (
                "ending in 400 is unverifiable when soft404 is off",
                chain_of(vec![deep_hop(301), root_hop(400)]),
                false,
                Expect::Is(unverifiable("400")),
            ),
            (
                "loop through the root",
                Err(FetchError::Loop(loop_chain())),
                true,
                Expect::Suspect,
            ),
            (
                "no location through the root",
                Err(FetchError::NoLocation(loop_chain())),
                true,
                Expect::Suspect,
            ),
            (
                "too many hops through the root",
                Err(FetchError::TooManyHops(loop_chain())),
                true,
                Expect::Suspect,
            ),
            (
                "failed chain through the root is unverifiable when soft404 is off",
                Err(FetchError::Loop(loop_chain())),
                false,
                Expect::Is(unverifiable("redirect loop")),
            ),
            (
                "failed chain that never visits the root",
                Err(FetchError::Loop(vec![deep_hop(301), deep_hop(301)])),
                true,
                Expect::Is(unverifiable("redirect loop")),
            ),
            (
                "a middle root hop keeps the permanent fix",
                chain_of(vec![
                    hop_at("/archives/123", 301),
                    hop_at("/?p=123", 301),
                    hop_at("/2020/post/", 200),
                ]),
                true,
                Expect::Permanent(hop_at("/2020/post/", 200).url),
            ),
            (
                "301 -> root 302 -> 200 is suspect",
                chain_of(vec![hop_at("/a", 301), hop_at("/", 302), hop_at("/b", 200)]),
                true,
                Expect::Suspect,
            ),
            (
                "302 -> root 301 -> 200 is temporary",
                chain_of(vec![hop_at("/a", 302), hop_at("/", 301), hop_at("/b", 200)]),
                true,
                Expect::Is(Verdict::Temporary {
                    chain: vec![hop_at("/a", 302), hop_at("/", 301), hop_at("/b", 200)],
                    final_url: hop_at("/b", 200).url,
                }),
            ),
            (
                "301 -> root 301 -> 400 is suspect",
                chain_of(vec![hop_at("/a", 301), hop_at("/", 301), hop_at("/x", 400)]),
                true,
                Expect::Suspect,
            ),
        ];
        for (name, outcome, soft404, expected) in cases {
            let chain = match &outcome {
                Ok(fetched) => fetched.chain.clone(),
                Err(
                    FetchError::Loop(c) | FetchError::NoLocation(c) | FetchError::TooManyHops(c),
                ) => c.clone(),
                Err(_) => Vec::new(),
            };
            let expected = match expected {
                Expect::Suspect => Verdict::Suspect {
                    chain,
                    reason: "redirect to root".to_owned(),
                },
                Expect::Permanent(fix) => Verdict::Permanent { chain, fix },
                Expect::Is(verdict) => verdict,
            };
            assert_eq!(
                classify(&outcome, &HostConfig { soft404 }),
                expected,
                "{name}"
            );
        }
    }

    /// `check_all` with a client built from `cfg`.
    async fn check_all_with_test_client(
        urls: Vec<Url>,
        cfg: &Config,
        deadline: Instant,
        on_done: impl FnMut(usize),
    ) -> HashMap<Url, Verdict> {
        let client = http_client(cfg).expect("the test client builds");
        check_all(urls, client, cfg, deadline, on_done).await
    }

    fn config_for(extra: &str) -> Config {
        config::parse(&format!(
            "site_url = \"https://site.example/\"\nuser_agent = \"rotguard-unit/1.0\"\nhost_interval_ms = 0\n{extra}"
        ))
        .expect("test config is valid")
    }

    /// Keeps a scripted server from spending replies on soft-404 probes.
    const NO_PROBE: &str = "[hosts.\"127.0.0.1\"]\nsoft404 = false";

    fn far() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    async fn gets(server: &MockServer) -> usize {
        server.received_requests().await.unwrap().len()
    }

    /// Answers each request from `replies` in turn (the last repeats), noting when it came.
    struct Script {
        replies: Vec<ResponseTemplate>,
        times: Arc<StdMutex<Vec<Instant>>>,
    }

    impl Respond for Script {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            let mut times = self.times.lock().unwrap();
            let reply = self.replies[times.len().min(self.replies.len() - 1)].clone();
            times.push(Instant::now());
            reply
        }
    }

    async fn scripted(replies: Vec<ResponseTemplate>) -> (MockServer, Arc<StdMutex<Vec<Instant>>>) {
        let server = MockServer::start().await;
        let times = Arc::new(StdMutex::new(Vec::new()));
        Mock::given(method("GET"))
            .respond_with(Script {
                replies,
                times: Arc::clone(&times),
            })
            .mount(&server)
            .await;
        (server, times)
    }

    fn unverifiable_reason(verdicts: &HashMap<Url, Verdict>, url: &Url) -> String {
        match &verdicts[url] {
            Verdict::Unverifiable { reason } => reason.clone(),
            other => panic!("expected unverifiable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_5xx_is_retried_twice() {
        let server = MockServer::start().await;
        Mock::given(path("/x"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let u = url(&server, "/x");
        let got = check_all_with_test_client(vec![u.clone()], &config_for(""), far(), |_| {}).await;
        assert_eq!(unverifiable_reason(&got, &u), "503");
        assert_eq!(gets(&server).await, 3);
    }

    #[tokio::test]
    async fn a_slow_handler_times_out_after_three_gets() {
        let server = MockServer::start().await;
        Mock::given(path("/x"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
            .mount(&server)
            .await;
        let u = url(&server, "/x");
        let got = check_all_with_test_client(
            vec![u.clone()],
            &config_for("timeout_ms = 100"),
            far(),
            |_| {},
        )
        .await;
        assert_eq!(unverifiable_reason(&got, &u), "timeout");
        assert_eq!(gets(&server).await, 3);
    }

    #[tokio::test]
    async fn a_429_is_retried_once_after_its_retry_after() {
        let (server, times) = scripted(vec![
            ResponseTemplate::new(429).insert_header("Retry-After", "1"),
            ResponseTemplate::new(200),
        ])
        .await;
        let u = url(&server, "/x");
        let got =
            check_all_with_test_client(vec![u.clone()], &config_for(NO_PROBE), far(), |_| {}).await;
        assert!(matches!(got[&u], Verdict::Ok { .. }), "{:?}", got[&u]);
        let times = times.lock().unwrap();
        assert_eq!(times.len(), 2);
        assert!(times[1] - times[0] >= Duration::from_secs(1));
    }

    #[tokio::test]
    async fn a_second_429_is_unverifiable_after_two_gets() {
        let server = MockServer::start().await;
        Mock::given(path("/x"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        let u = url(&server, "/x");
        let got = check_all_with_test_client(vec![u.clone()], &config_for(""), far(), |_| {}).await;
        assert_eq!(unverifiable_reason(&got, &u), "429");
        assert_eq!(gets(&server).await, 2);
    }

    #[tokio::test]
    async fn a_404_then_a_200_is_ok() {
        let (server, times) =
            scripted(vec![ResponseTemplate::new(404), ResponseTemplate::new(200)]).await;
        let u = url(&server, "/x");
        let got = check_all_with_test_client(
            vec![u.clone()],
            &config_for(&format!("recheck_delay_ms = 50\n{NO_PROBE}")),
            far(),
            |_| {},
        )
        .await;
        assert!(matches!(got[&u], Verdict::Ok { .. }), "{:?}", got[&u]);
        assert_eq!(times.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_404_twice_is_dead_after_the_recheck_delay() {
        let (server, times) = scripted(vec![ResponseTemplate::new(404)]).await;
        let u = url(&server, "/x");
        let got = check_all_with_test_client(
            vec![u.clone()],
            &config_for("recheck_delay_ms = 300"),
            far(),
            |_| {},
        )
        .await;
        assert_eq!(
            got[&u],
            Verdict::Dead {
                reason: "404".to_owned()
            }
        );
        let times = times.lock().unwrap();
        assert_eq!(times.len(), 2);
        assert!(times[1] - times[0] >= Duration::from_millis(300));
    }

    #[tokio::test]
    async fn a_hanging_handler_is_unverifiable_at_the_deadline() {
        let server = MockServer::start().await;
        Mock::given(path("/x"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
            .mount(&server)
            .await;
        let u = url(&server, "/x");
        let started = Instant::now();
        let got = check_all_with_test_client(
            vec![u.clone()],
            &config_for("timeout_ms = 30000"),
            started + Duration::from_secs(1),
            |_| {},
        )
        .await;
        assert_eq!(unverifiable_reason(&got, &u), "deadline");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn every_url_gets_a_verdict() {
        let server = MockServer::start().await;
        Mock::given(path("/a"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(path("/b"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let (a, b) = (url(&server, "/a"), url(&server, "/b"));
        let got =
            check_all_with_test_client(vec![a.clone(), b.clone()], &config_for(""), far(), |_| {})
                .await;
        assert_eq!(got.len(), 2);
        assert!(matches!(got[&a], Verdict::Ok { .. }));
        assert_eq!(unverifiable_reason(&got, &b), "403");
    }

    #[tokio::test]
    async fn bot_wall_probe_decides_blocked() {
        for (status, sibling, blocked) in [(403, 403, true), (403, 404, false), (429, 429, false)] {
            let server = MockServer::start().await;
            Mock::given(path("/w/page"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            mount(&server, "^/w/.+", ResponseTemplate::new(sibling)).await;
            let page = url(&server, "/w/page");
            let got =
                check_all_with_test_client(vec![page.clone()], &config_for(""), far(), |_| {})
                    .await;
            if blocked {
                assert_eq!(
                    got[&page],
                    Verdict::Blocked { status },
                    "{status}/{sibling}"
                );
            } else {
                assert_eq!(
                    unverifiable_reason(&got, &page),
                    status.to_string(),
                    "{status}/{sibling}"
                );
            }
        }
    }

    #[tokio::test]
    async fn on_done_counts_each_url_as_it_finishes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let urls: Vec<Url> = ["/a", "/b", "/c"].iter().map(|p| url(&server, p)).collect();
        let mut counts = Vec::new();
        check_all_with_test_client(urls, &config_for(NO_PROBE), far(), |done| counts.push(done))
            .await;
        assert_eq!(counts, [1, 2, 3]);
    }

    fn page(title: &str, body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_string(format!(
            "<html><head><title>{title}</title></head><body>{body}</body></html>"
        ))
    }

    async fn mount(server: &MockServer, route: &str, response: ResponseTemplate) {
        Mock::given(path_regex(route))
            .respond_with(response)
            .mount(server)
            .await;
    }

    fn reason_of(verdict: &Verdict) -> &str {
        match verdict {
            Verdict::Suspect { reason, .. } => reason,
            other => panic!("expected suspect, got {other:?}"),
        }
    }

    #[test]
    fn identical_text_has_similarity_one_and_unrelated_text_less_than_point_nine() {
        let a = shingles("<p>the quick brown fox jumps over the lazy dog</p>");
        assert!((jaccard(&a, &a) - 1.0).abs() < f64::EPSILON);
        let b = shingles("<p>pack my box with five dozen liquor jugs today</p>");
        assert!(jaccard(&a, &b) < 0.9);
        assert!(jaccard(&HashSet::new(), &HashSet::new()) < f64::EPSILON);
    }

    #[test]
    fn visible_text_skips_scripts_styles_and_tags() {
        let html = "<style>a b c d</style><script>e f g h</script><p>One &amp;&nbsp;Two, three</p>";
        let got = shingles(html);
        let expected: HashSet<String> = ["one two three"].map(String::from).into();
        assert_eq!(got, expected);
    }

    #[test]
    fn an_unclosed_block_hides_the_rest_of_the_body() {
        let got = shingles("<p>one two three</p><!-- four five six");
        let expected: HashSet<String> = ["one two three"].map(String::from).into();
        assert_eq!(got, expected);
        assert!(shingles("<SCRIPT>a b c d").is_empty());
    }

    #[test]
    fn hostile_body_is_stripped_in_linear_time() {
        let html = "<script></script><!--".repeat(256 * 1024 / 21);
        let started = std::time::Instant::now();
        let got = shingles(&html);
        assert!(got.is_empty());
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn many_closed_blocks_are_stripped_in_linear_time() {
        let html = "<!---->".repeat(256 * 1024 / 7);
        let started = std::time::Instant::now();
        assert!(shingles(&html).is_empty());
        assert!(started.elapsed() < Duration::from_millis(500));
        let words = shingles("a <script b>x</script> b <style>y</style> c <scripty> d <!--z--> e");
        let expected: HashSet<String> = ["a b c", "b c d", "c d e"].map(String::from).into();
        assert_eq!(words, expected);
    }

    #[test]
    fn title_is_extracted_trimmed_and_collapsed() {
        assert_eq!(
            title_of("<HEAD><Title lang=en>\n  Docs \t home \n</TITLE>"),
            "Docs home"
        );
        assert_eq!(title_of("<p>no title</p>"), "");
    }

    #[test]
    fn tokens_are_24_lowercase_alphanumerics_and_differ() {
        let (a, b) = (random_token(), random_token());
        let ok = |t: &str| {
            t.len() == 24
                && t.bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        };
        assert!(ok(&a) && ok(&b), "{a} {b}");
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn two_urls_under_one_path_share_one_probe_that_404s() {
        let server = MockServer::start().await;
        mount(&server, "^/docs/(a|b)$", page("T", "some page text here")).await;
        let (a, b) = (url(&server, "/docs/a"), url(&server, "/docs/b"));
        let got =
            check_all_with_test_client(vec![a.clone(), b.clone()], &config_for(""), far(), |_| {})
                .await;
        assert!(matches!(got[&a], Verdict::Ok { .. }), "{:?}", got[&a]);
        assert!(matches!(got[&b], Verdict::Ok { .. }), "{:?}", got[&b]);
        let probes: Vec<_> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| !matches!(r.url.path(), "/docs/a" | "/docs/b"))
            .collect();
        assert_eq!(probes.len(), 1, "{probes:?}");
        let token = probes[0]
            .url
            .path()
            .strip_prefix("/docs/")
            .unwrap()
            .to_owned();
        assert_eq!(token.len(), 24);
    }

    #[tokio::test]
    async fn a_url_ending_in_slash_gets_one_sibling_probe_ending_in_slash() {
        let server = MockServer::start().await;
        mount(&server, "^/docs/real/$", page("T", "some page text here")).await;
        let target = url(&server, "/docs/real/");
        let got =
            check_all_with_test_client(vec![target.clone()], &config_for(""), far(), |_| {}).await;
        assert!(
            matches!(got[&target], Verdict::Ok { .. }),
            "{:?}",
            got[&target]
        );
        let probes: Vec<_> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.url.path() != "/docs/real/")
            .collect();
        assert_eq!(probes.len(), 1, "{probes:?}");
        let path = probes[0].url.path();
        let token = path
            .strip_prefix("/docs/")
            .and_then(|rest| rest.strip_suffix('/'))
            .unwrap_or_else(|| panic!("unexpected probe path {path}"));
        assert_eq!(token.len(), 24, "{path}");
    }

    /// Label, routes, config, requested path, suspect reason (None for ok), request count.
    type ProbeCase<'a> = (
        &'a str,
        Vec<(&'a str, ResponseTemplate)>,
        &'a str,
        &'a str,
        Option<&'a str>,
        Option<usize>,
    );

    #[tokio::test]
    async fn soft404_probe_verdicts() {
        let landing = || ResponseTemplate::new(302).insert_header("location", "/docs/landing");
        let docs_html = "<html><head><title>Docs home</title></head><body>alpha beta gamma delta epsilon zeta</body></html>";
        let words = (0..400)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let not_found_page = || page("Page Not Found \u{2013} Example", "sorry");
        let unrelated = || {
            page(
                "Unrelated heading",
                "completely different words appear here",
            )
        };
        // `n` words `{tag}0..` that no other tag shares.
        let run = |tag: &str, n: usize| {
            (0..n)
                .map(|i| format!("{tag}{i}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        // Pages titled "Shared" (1 word, so a body of n words has n - 1 shingles) whose
        // bodies share `shared` words and then differ for `own` words each.
        let overlap = |shared: usize, own: usize| {
            (
                page("Shared", &format!("{} {}", run("s", shared), run("a", own))),
                page("Shared", &format!("{} {}", run("s", shared), run("b", own))),
            )
        };
        // (label, routes, config, requested path, expected suspect reason or None for ok,
        // expected request count)
        let cases: Vec<ProbeCase> = vec![
            (
                "probe with the same title",
                vec![
                    ("^/docs/real$", page("Docs home", "alpha beta gamma delta")),
                    (
                        "^/docs/[a-z0-9]{24}$",
                        page("Docs home", "one two three four five"),
                    ),
                ],
                "",
                "/docs/real",
                Some("probe: same title"),
                None,
            ),
            (
                "root with the probe's title but unrelated text is no control",
                vec![
                    ("^/blog/gone$", page("Oops", &run("g", 200))),
                    ("^/blog/[a-z0-9]{24}$", page("Oops", &run("g", 200))),
                    ("^/$", page("Oops", &run("r", 200))),
                ],
                "",
                "/blog/gone",
                Some("probe: same title"),
                None,
            ),
            (
                "same title, different text, 20 shingles each",
                vec![
                    ("^/t/a$", page("Shared", &run("a", 21))),
                    ("^/t/[a-z0-9]{24}$", page("Shared", &run("b", 21))),
                ],
                "",
                "/t/a",
                None,
                None,
            ),
            (
                "same title, different text, 19 shingles in the link",
                vec![
                    ("^/t/a$", page("Shared", &run("a", 20))),
                    ("^/t/[a-z0-9]{24}$", page("Shared", &run("b", 21))),
                ],
                "",
                "/t/a",
                Some("probe: same title"),
                None,
            ),
            (
                "same title, different text, 19 shingles in the probe",
                vec![
                    ("^/t/a$", page("Shared", &run("a", 21))),
                    ("^/t/[a-z0-9]{24}$", page("Shared", &run("b", 20))),
                ],
                "",
                "/t/a",
                Some("probe: same title"),
                None,
            ),
            (
                "same title, Jaccard just under 0.5",
                {
                    let (link, probe) = overlap(20, 10);
                    vec![("^/t/a$", link), ("^/t/[a-z0-9]{24}$", probe)]
                },
                "",
                "/t/a",
                None,
                None,
            ),
            (
                "same title, Jaccard exactly 0.5",
                {
                    let (link, probe) = overlap(21, 10);
                    vec![("^/t/a$", link), ("^/t/[a-z0-9]{24}$", probe)]
                },
                "",
                "/t/a",
                Some("probe: same title"),
                None,
            ),
            (
                "probe ending at the same final url",
                vec![
                    ("^/docs/real$", landing()),
                    ("^/docs/[a-z0-9]{24}$", landing()),
                    ("^/docs/landing$", page("Landing", "alpha beta gamma delta")),
                ],
                "",
                "/docs/real",
                Some("probe: same final URL"),
                None,
            ),
            (
                "probe redirected to the unredirected link is no evidence",
                vec![
                    ("^/docs/real$", page("Docs home", "alpha beta gamma delta")),
                    (
                        "^/docs/[a-z0-9]{24}$",
                        ResponseTemplate::new(302).insert_header("location", "/docs/real"),
                    ),
                ],
                "",
                "/docs/real",
                None,
                None,
            ),
            (
                "404 probe with the page's title and text is ignored",
                vec![
                    (
                        "^/docs/real$",
                        page("Docs home", "alpha beta gamma delta epsilon zeta"),
                    ),
                    (
                        "^/docs/[a-z0-9]{24}$",
                        ResponseTemplate::new(404).set_body_string(docs_html),
                    ),
                ],
                "",
                "/docs/real",
                None,
                None,
            ),
            (
                "root url with a not-found title needs no probe",
                vec![("^/$", page("Page not found", "sorry"))],
                "",
                "/",
                Some("title says \"not found\""),
                Some(1),
            ),
            (
                "similar body with another title",
                vec![
                    ("^/x/a$", page("Real", &words)),
                    ("^/x/[a-z0-9]{24}$", page("Other", &words)),
                ],
                "",
                "/x/a",
                Some("probe: similar body"),
                None,
            ),
            (
                "not-found title with a 200 probe",
                vec![
                    ("^/z/a$", not_found_page()),
                    ("^/z/[a-z0-9]{24}$", unrelated()),
                ],
                "",
                "/z/a",
                Some("title says \"not found\""),
                None,
            ),
            (
                "phrase in the title is ignored when the probe is 404",
                vec![("^/w/a$", page("404 errors explained", "an article"))],
                "",
                "/w/a",
                None,
                None,
            ),
            (
                "phrase in the title is ignored when soft404 is off",
                vec![
                    ("^/z/a$", not_found_page()),
                    ("^/z/[a-z0-9]{24}$", unrelated()),
                ],
                NO_PROBE,
                "/z/a",
                None,
                None,
            ),
            (
                "not-found title on a page that needs JavaScript stays suspect",
                vec![
                    (
                        "^/z/a$",
                        page("Not Found", "<p>Please enable JavaScript to continue</p>"),
                    ),
                    (
                        "^/z/[a-z0-9]{24}$",
                        page("Not Found", "<p>Please enable JavaScript to continue</p>"),
                    ),
                ],
                "",
                "/z/a",
                Some("title says \"not found\""),
                None,
            ),
            (
                "failed probe does not hide a not-found title",
                vec![
                    ("^/z/a$", page("Not Found", "sorry")),
                    ("^/z/[a-z0-9]{24}$", ResponseTemplate::new(500)),
                ],
                "",
                "/z/a",
                Some("title says \"not found\""),
                None,
            ),
            (
                "soft404 off sends no probe",
                vec![
                    ("^/docs/real$", page("Docs home", "alpha")),
                    ("^/docs/[a-z0-9]{24}$", page("Docs home", "alpha")),
                ],
                NO_PROBE,
                "/docs/real",
                None,
                Some(1),
            ),
        ];
        for (name, routes, extra, target, reason, requests) in cases {
            let server = MockServer::start().await;
            for (route, response) in routes {
                mount(&server, route, response).await;
            }
            let u = url(&server, target);
            let got =
                check_all_with_test_client(vec![u.clone()], &config_for(extra), far(), |_| {})
                    .await;
            match reason {
                Some(reason) => assert_eq!(reason_of(&got[&u]), reason, "{name}"),
                None => assert!(
                    matches!(got[&u], Verdict::Ok { .. }),
                    "{name}: {:?}",
                    got[&u]
                ),
            }
            if let Some(requests) = requests {
                assert_eq!(gets(&server).await, requests, "{name}");
            }
        }
    }
}
