//! Wayback CDX lookups: the latest status-200 capture on or before a date, else the earliest
//! after it, with every request spaced through one shared gate.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use chrono::NaiveDate;
use reqwest::Client;
use tokio::sync::Mutex;
use tokio::time::{Instant, sleep_until, timeout_at};
use url::Url;

use crate::config::Config;

/// Attempts per CDX query.
const ATTEMPTS: usize = 3;

/// Why no archived copy could be proposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoFix {
    /// The archive has no status-200 capture.
    NoCapture,
    /// The archive did not give a usable answer in 3 attempts.
    Unreachable,
    /// The run's deadline passed first.
    Deadline,
}

impl fmt::Display for NoFix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoCapture => "no archived copy",
            Self::Unreachable => "archive unreachable",
            Self::Deadline => "deadline",
        })
    }
}

impl Error for NoFix {}

/// Which side of the date a query asks for.
#[derive(Debug, Clone, Copy)]
enum Side {
    /// The latest capture on or before the date.
    OnOrBefore,
    /// The earliest capture after the date.
    After,
}

/// Looks up Wayback snapshots through one shared gate.
#[derive(Debug)]
pub struct Archive {
    client: Client,
    base: String,
    /// The CDX search endpoint, without a query.
    endpoint: Url,
    interval: Duration,
    /// When the last archive request started.
    last_start: Mutex<Option<Instant>>,
}

/// Whether URL parsing would change what `original` points at: a `\`, a tab or newline, a
/// trailing space or control character (stripped), or a dot segment (`.`, `..`, `%2e` in any
/// case) in the path before any `?`.
fn parsing_changes_target(original: &str) -> bool {
    if original.contains(['\\', '\t', '\n', '\r']) || original.ends_with(|c: char| c <= ' ') {
        return true;
    }
    let path = original.split('?').next().unwrap_or_default();
    path.split('/').any(|segment| {
        let segment = segment.to_ascii_lowercase().replace("%2e", ".");
        segment == "." || segment == ".."
    })
}

impl Archive {
    /// Builds the client once from the config's User-Agent and timeout.
    ///
    /// # Panics
    /// If the TLS backend cannot initialise, which is a broken build or platform. A
    /// `user_agent` from a parsed `Config` is already checked as a valid header value.
    #[must_use]
    pub fn new(cfg: &Config) -> Archive {
        let client = Client::builder()
            .timeout(cfg.timeout)
            .user_agent(cfg.user_agent.as_str())
            .build()
            .expect("user_agent is validated by config::parse; the TLS backend is built in");
        let base = cfg.archive_url.as_str().trim_end_matches('/').to_owned();
        let endpoint = Url::parse(&format!("{base}/cdx/search/cdx"))
            .expect("a parsed archive_url plus a fixed path is a valid URL");
        Archive {
            client,
            base,
            endpoint,
            interval: cfg.archive_interval,
            last_start: Mutex::new(None),
        }
    }

    /// The archived copy of `url` nearest `date`, without a fragment.
    pub async fn snapshot(
        &self,
        url: &Url,
        date: NaiveDate,
        deadline: Instant,
    ) -> Result<Url, NoFix> {
        timeout_at(deadline, self.lookup(url, date))
            .await
            .unwrap_or(Err(NoFix::Deadline))
    }

    async fn lookup(&self, url: &Url, date: NaiveDate) -> Result<Url, NoFix> {
        let day = date.format("%Y%m%d").to_string();
        let capture = match self.query(url, &day, Side::OnOrBefore).await? {
            Some(capture) => capture,
            None => self
                .query(url, &day, Side::After)
                .await?
                .ok_or(NoFix::NoCapture)?,
        };
        // A capture the archive reports but we cannot turn into a URL is an unusable answer.
        let snapshot = Url::parse(&format!(
            "{}/web/{}/{}",
            self.base, capture.timestamp, capture.original
        ))
        .map_err(|_| NoFix::Unreachable)?;
        // Parsing resolves dot segments, turns `\` into `/`, drops tabs and newlines and splits
        // off a `#`; each would point the fix somewhere else. Mere percent-encoding is fine.
        if snapshot.fragment().is_some() || parsing_changes_target(&capture.original) {
            Err(NoFix::Unreachable)
        } else {
            Ok(snapshot)
        }
    }

    /// One CDX query with up to 3 attempts.
    async fn query(&self, url: &Url, day: &str, side: Side) -> Result<Option<Capture>, NoFix> {
        let (bound, limit) = match side {
            Side::OnOrBefore => ("to", "-1"),
            Side::After => ("from", "1"),
        };
        for _ in 0..ATTEMPTS {
            self.wait_for_gate().await;
            let mut request = self.endpoint.clone();
            request.query_pairs_mut().extend_pairs([
                ("url", url.as_str()),
                ("filter", "statuscode:200"),
                ("fl", "timestamp,original"),
                ("output", "json"),
                (bound, day),
                ("limit", limit),
            ]);
            let sent = self.client.get(request).send().await;
            let Ok(response) = sent else { continue };
            if !response.status().is_success() {
                continue;
            }
            // A body that fails to arrive or to parse is a failed attempt like any other.
            let Ok(bytes) = response.bytes().await else {
                continue;
            };
            if let Ok(body) = std::str::from_utf8(&bytes)
                && let Some(rows) = parse_rows(body)
            {
                return Ok(rows.into_iter().next_back());
            }
        }
        Err(NoFix::Unreachable)
    }

    /// Waits until `interval` has passed since the last request started, then claims the slot.
    async fn wait_for_gate(&self) {
        let mut last = self.last_start.lock().await;
        if let Some(previous) = *last {
            sleep_until(previous + self.interval).await;
        }
        *last = Some(Instant::now());
    }
}

/// One CDX row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Capture {
    timestamp: String,
    original: String,
}

/// Parses `[["timestamp","original"],["2020...","https://..."],...]` or `[]`.
///
/// Drops the header row. `None` when the text is not that shape.
fn parse_rows(text: &str) -> Option<Vec<Capture>> {
    let mut chars = text.chars().peekable();
    let mut rows: Vec<Vec<String>> = Vec::new();
    expect(&mut chars, '[')?;
    if skip_ws_peek(&mut chars)? != ']' {
        loop {
            rows.push(parse_row(&mut chars)?);
            match skip_ws_peek(&mut chars)? {
                ',' => {
                    chars.next();
                }
                ']' => break,
                _ => return None,
            }
        }
    }
    expect(&mut chars, ']')?;
    if chars.any(|c| !c.is_whitespace()) {
        return None;
    }
    let mut captures = Vec::with_capacity(rows.len());
    for row in rows {
        let [timestamp, original] = <[String; 2]>::try_from(row).ok()?;
        if timestamp == "timestamp" {
            continue;
        }
        if timestamp.is_empty() || !timestamp.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        captures.push(Capture {
            timestamp,
            original,
        });
    }
    Some(captures)
}

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

fn skip_ws_peek(chars: &mut Chars<'_>) -> Option<char> {
    while chars.peek()?.is_whitespace() {
        chars.next();
    }
    chars.peek().copied()
}

fn expect(chars: &mut Chars<'_>, want: char) -> Option<()> {
    (skip_ws_peek(chars)? == want).then(|| {
        chars.next();
    })
}

fn parse_row(chars: &mut Chars<'_>) -> Option<Vec<String>> {
    expect(chars, '[')?;
    let mut cells = Vec::new();
    loop {
        cells.push(parse_string(chars)?);
        match skip_ws_peek(chars)? {
            ',' => {
                chars.next();
            }
            ']' => {
                chars.next();
                return Some(cells);
            }
            _ => return None,
        }
    }
}

fn parse_string(chars: &mut Chars<'_>) -> Option<String> {
    expect(chars, '"')?;
    let mut out = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => out.push(match chars.next()? {
                '"' => '"',
                '\\' => '\\',
                '/' => '/',
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                'b' => '\u{8}',
                'f' => '\u{c}',
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    // A lone surrogate is not a char; treat the answer as unusable.
                    char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?
                }
                _ => return None,
            }),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex as StdMutex};

    use super::*;
    use crate::config;
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    type Starts = Arc<StdMutex<Vec<std::time::Instant>>>;

    fn archive(server: &MockServer, extra: &str) -> Archive {
        let text = format!(
            "site_url = \"https://site.example/\"\nuser_agent = \"rotguard-unit/1.0\"\n\
             archive_url = \"{}\"\n{extra}",
            server.uri()
        );
        Archive::new(&config::parse(&text).expect("test config is valid"))
    }

    fn target() -> Url {
        Url::parse("https://old.example/a").expect("literal URL")
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(30)
    }

    /// Emulates CDX `to`/`from`/`limit` over fixed `(timestamp, original)` rows.
    struct Cdx {
        rows: Vec<(&'static str, &'static str)>,
        starts: Starts,
    }

    impl Respond for Cdx {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            self.starts
                .lock()
                .expect("starts lock")
                .push(std::time::Instant::now());
            let q: HashMap<_, _> = request.url.query_pairs().into_owned().collect();
            assert_eq!(q["url"], "https://old.example/a");
            assert_eq!(q["filter"], "statuscode:200");
            assert_eq!(q["fl"], "timestamp,original");
            assert_eq!(q["output"], "json");
            let mut rows: Vec<_> = self
                .rows
                .iter()
                .filter(|(ts, _)| q.get("to").is_none_or(|to| ts[..8] <= to[..]))
                .filter(|(ts, _)| q.get("from").is_none_or(|from| ts[..8] >= from[..]))
                .collect();
            rows.sort();
            let rows = match q["limit"].as_str() {
                "-1" => rows.split_off(rows.len().saturating_sub(1)),
                "1" => rows.into_iter().take(1).collect(),
                other => panic!("unexpected limit {other}"),
            };
            let body = if rows.is_empty() {
                "[]".to_owned()
            } else {
                let cells: Vec<String> = rows
                    .iter()
                    .map(|(ts, orig)| {
                        // Rows are written as JSON, so a backslash must be escaped.
                        let orig = orig.replace('\\', "\\\\");
                        format!("[\"{ts}\",\"{orig}\"]")
                    })
                    .collect();
                format!("[[\"timestamp\",\"original\"],{}]", cells.join(","))
            };
            ResponseTemplate::new(200).set_body_string(body)
        }
    }

    async fn serve(rows: Vec<(&'static str, &'static str)>) -> (MockServer, Starts) {
        let server = MockServer::start().await;
        let starts = Starts::default();
        Mock::given(any())
            .respond_with(Cdx {
                rows,
                starts: Arc::clone(&starts),
            })
            .mount(&server)
            .await;
        (server, starts)
    }

    #[tokio::test]
    async fn a_capture_before_the_date_wins_over_one_after() {
        let (server, _) = serve(vec![
            ("20200101000000", "https://old.example/a"),
            ("20220101000000", "https://old.example/a"),
        ])
        .await;
        let got = archive(&server, "")
            .snapshot(&target(), day(2021, 6, 1), soon())
            .await;
        let want = format!("{}/web/20200101000000/https://old.example/a", server.uri());
        assert_eq!(got, Ok(Url::parse(&want).expect("valid")));
    }

    #[tokio::test]
    async fn only_a_later_capture_is_the_earliest_after_the_date() {
        let (server, _) = serve(vec![
            ("20230101000000", "https://old.example/a"),
            ("20220101000000", "https://old.example/a"),
        ])
        .await;
        let got = archive(&server, "")
            .snapshot(&target(), day(2021, 6, 1), soon())
            .await;
        let want = format!("{}/web/20220101000000/https://old.example/a", server.uri());
        assert_eq!(got, Ok(Url::parse(&want).expect("valid")));
    }

    #[tokio::test]
    async fn the_result_uses_the_captures_original() {
        let (server, _) = serve(vec![("20200101000000", "http://old.example/a")]).await;
        let got = archive(&server, "")
            .snapshot(&target(), day(2021, 6, 1), soon())
            .await
            .expect("a capture exists");
        assert!(
            got.as_str()
                .ends_with("/web/20200101000000/http://old.example/a")
        );
        assert_eq!(got.fragment(), None);
    }

    #[tokio::test]
    async fn an_original_that_url_rewrites_is_unreachable() {
        for original in [
            "https://a.test/a/../b",
            "https://a.test/a/%2e%2e/b",
            "https://a.test/a#x",
            "https://a.test/../../../x",
            "https://a.test/a\\b",
            "https://a.test/a/.",
            "https://a.test/a/%2E%2e/b",
            "https://a.test/a/.%2E/b",
            "https://a.test/a ",
        ] {
            let (server, _) = serve(vec![("20200101000000", original)]).await;
            let got = archive(&server, "")
                .snapshot(&target(), day(2021, 6, 1), soon())
                .await;
            assert_eq!(got, Err(NoFix::Unreachable), "{original}");
        }
    }

    #[tokio::test]
    async fn an_original_that_only_needs_encoding_keeps_its_snapshot() {
        for original in [
            "https://a.test:80/a",
            "HTTPS://A.TEST/a",
            "https://a.test/a?q=1",
            "https://a.test/a?",
            "https://a.test/a b",
            "https://a.test/café",
        ] {
            let (server, _) = serve(vec![("20200101000000", original)]).await;
            let got = archive(&server, "")
                .snapshot(&target(), day(2021, 6, 1), soon())
                .await;
            assert!(got.is_ok(), "{original}: {got:?}");
        }
    }

    #[tokio::test]
    async fn no_captures_is_no_capture() {
        let (server, starts) = serve(Vec::new()).await;
        let got = archive(&server, "archive_interval_ms = 1\n")
            .snapshot(&target(), day(2021, 6, 1), soon())
            .await;
        assert_eq!(got, Err(NoFix::NoCapture));
        assert_eq!(starts.lock().expect("lock").len(), 2, "both queries ran");
    }

    #[tokio::test]
    async fn a_503_archive_is_unreachable_after_exactly_three_requests() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let got = archive(&server, "archive_interval_ms = 1\n")
            .snapshot(&target(), day(2021, 6, 1), soon())
            .await;
        assert_eq!(got, Err(NoFix::Unreachable));
        assert_eq!(server.received_requests().await.expect("recorded").len(), 3);
    }

    #[tokio::test]
    async fn an_unparseable_200_counts_as_a_failure() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>busy</html>"))
            .mount(&server)
            .await;
        let got = archive(&server, "archive_interval_ms = 1\n")
            .snapshot(&target(), day(2021, 6, 1), soon())
            .await;
        assert_eq!(got, Err(NoFix::Unreachable));
        assert_eq!(server.received_requests().await.expect("recorded").len(), 3);
    }

    #[tokio::test]
    async fn requests_start_at_least_the_interval_apart() {
        let (server, starts) = serve(Vec::new()).await;
        let archive = archive(&server, "archive_interval_ms = 200\n");
        let url = target();
        let date = day(2021, 6, 1);
        let (a, b, c) = tokio::join!(
            archive.snapshot(&url, date, soon()),
            archive.snapshot(&url, date, soon()),
            archive.snapshot(&url, date, soon()),
        );
        for lookup in [a, b, c] {
            assert_eq!(lookup, Err(NoFix::NoCapture));
        }
        let mut times = starts.lock().expect("lock").clone();
        times.sort();
        assert_eq!(times.len(), 6);
        for pair in times.windows(2) {
            // Allow a little slack between the client's gate and the server's stamps.
            assert!(
                pair[1] - pair[0] >= Duration::from_millis(150),
                "starts only {:?} apart",
                pair[1] - pair[0]
            );
        }
    }

    #[tokio::test]
    async fn a_hanging_archive_hits_the_deadline() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
            .mount(&server)
            .await;
        let started = Instant::now();
        let got = archive(&server, "timeout_ms = 20000\n")
            .snapshot(
                &target(),
                day(2021, 6, 1),
                Instant::now() + Duration::from_secs(1),
            )
            .await;
        assert_eq!(got, Err(NoFix::Deadline));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn the_parser_handles_escapes_and_rejects_other_shapes() {
        let rows =
            parse_rows(r#"[["timestamp","original"],["2020","http://x/a\"b\\c"]]"#).expect("valid");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].original, r#"http://x/a"b\c"#);
        assert_eq!(parse_rows("[]"), Some(Vec::new()));
        assert_eq!(parse_rows("{}"), None);
        assert_eq!(parse_rows(r#"[["2020"]]"#), None);
        assert_eq!(parse_rows(r#"[["abc","x"]]"#), None);
        assert_eq!(parse_rows("[] junk"), None);
    }
}
