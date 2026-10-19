//! Fixtures shared by the acceptance tests: a temp site, mock servers `M` (the linked site) and
//! `A` (the Wayback CDX API), and a helper that runs the binary.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use tempfile::TempDir;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// Slack for comparing arrival times at a mock against start times in the client.
pub const JITTER: Duration = Duration::from_millis(40);

/// The User-Agent every fixture config sets.
pub const USER_AGENT: &str = "rotguard-test/1.0";

// ---------------------------------------------------------------------------------------------
// Request log
// ---------------------------------------------------------------------------------------------

/// One request a mock saw.
#[derive(Debug, Clone)]
pub struct Req {
    pub path: String,
    pub method: String,
    pub user_agent: Option<String>,
    pub at: Instant,
}

/// Shared, ordered record of the requests a mock received.
#[derive(Debug, Clone, Default)]
pub struct Log(Arc<Mutex<Vec<Req>>>);

impl Log {
    fn push(&self, request: &Request) {
        let req = Req {
            path: request.url.path().to_owned(),
            method: request.method.to_string(),
            user_agent: request
                .headers
                .get("user-agent")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned),
            at: Instant::now(),
        };
        self.0.lock().expect("log lock").push(req);
    }

    /// Every request, in arrival order.
    pub fn all(&self) -> Vec<Req> {
        let mut all = self.0.lock().expect("log lock").clone();
        all.sort_by_key(|r| r.at);
        all
    }

    /// Requests for exactly `path`.
    pub fn hits(&self, path: &str) -> Vec<Req> {
        self.all().into_iter().filter(|r| r.path == path).collect()
    }

    /// Requests that are not soft-404 probes.
    pub fn own(&self) -> Vec<Req> {
        self.all()
            .into_iter()
            .filter(|r| !is_probe(&r.path))
            .collect()
    }

    /// Soft-404 probe requests.
    pub fn probes(&self) -> Vec<Req> {
        self.all()
            .into_iter()
            .filter(|r| is_probe(&r.path))
            .collect()
    }
}

/// A probe's last path segment is 24 characters of `[a-z0-9]`.
pub fn is_probe(path: &str) -> bool {
    let last = path.rsplit('/').next().unwrap_or("");
    last.len() == 24
        && last
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Asserts that consecutive arrival times are at least `min` apart (within `JITTER`).
pub fn assert_gaps(reqs: &[Req], min: Duration) {
    for pair in reqs.windows(2) {
        let gap = pair[1].at.duration_since(pair[0].at);
        assert!(
            gap + JITTER >= min,
            "requests {} and {} arrived {gap:?} apart, expected at least {min:?}",
            pair[0].path,
            pair[1].path
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Mock M: a routing table behind one recording responder
// ---------------------------------------------------------------------------------------------

type Handler = Box<dyn Fn(&Request, usize) -> ResponseTemplate + Send + Sync>;

enum Pattern {
    Exact(String),
    Regex(Regex),
}

struct Route {
    pattern: Pattern,
    hits: AtomicUsize,
    handler: Handler,
}

/// A routing table. The first route whose pattern matches the request path answers; a handler
/// gets the request and the number of earlier hits on its route. Unmatched paths answer 404.
#[derive(Default)]
pub struct Routes(Vec<Route>);

impl Routes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers `path` with a handler.
    pub fn with(
        mut self,
        path: &str,
        handler: impl Fn(&Request, usize) -> ResponseTemplate + Send + Sync + 'static,
    ) -> Self {
        self.0.push(Route {
            pattern: Pattern::Exact(path.to_owned()),
            hits: AtomicUsize::new(0),
            handler: Box::new(handler),
        });
        self
    }

    /// Answers `path` with the same response every time.
    pub fn fixed(self, path: &str, response: ResponseTemplate) -> Self {
        self.with(path, move |_, _| response.clone())
    }

    /// Answers every path matching the regex with the same response every time.
    pub fn fixed_re(mut self, pattern: &str, response: ResponseTemplate) -> Self {
        self.0.push(Route {
            pattern: Pattern::Regex(Regex::new(pattern).expect("test regex")),
            hits: AtomicUsize::new(0),
            handler: Box::new(move |_, _| response.clone()),
        });
        self
    }
}

struct Router {
    log: Log,
    routes: Routes,
}

impl Respond for Router {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.log.push(request);
        let path = request.url.path();
        for route in &self.routes.0 {
            let matched = match &route.pattern {
                Pattern::Exact(p) => p == path,
                Pattern::Regex(re) => re.is_match(path),
            };
            if matched {
                let n = route.hits.fetch_add(1, Ordering::SeqCst);
                return (route.handler)(request, n);
            }
        }
        status(404)
    }
}

/// 200 with an HTML page.
pub fn page(title: &str, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/html")
        .set_body_string(format!(
            "<html><head><title>{title}</title></head><body><p>{body}</p></body></html>"
        ))
}

/// A redirect with the given status.
pub fn redirect(code: u16, to: &str) -> ResponseTemplate {
    ResponseTemplate::new(code).insert_header("location", to)
}

/// An empty response with the given status.
pub fn status(code: u16) -> ResponseTemplate {
    ResponseTemplate::new(code)
}

// ---------------------------------------------------------------------------------------------
// Mock A: Wayback CDX API
// ---------------------------------------------------------------------------------------------

/// One archived capture. `url` is what a query matches; `original` is what the row reports.
#[derive(Debug, Clone)]
pub struct Capture {
    timestamp: String,
    url: String,
    original: String,
    status: u16,
}

impl Capture {
    pub fn of(timestamp: &str, url: &str) -> Self {
        Self::with_original(timestamp, url, url)
    }

    pub fn with_original(timestamp: &str, url: &str, original: &str) -> Self {
        Self {
            timestamp: timestamp.to_owned(),
            url: url.to_owned(),
            original: original.to_owned(),
            status: 200,
        }
    }

    /// The same capture, archived with another HTTP status.
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
}

#[derive(Debug, Clone)]
enum CdxMode {
    Serve,
    Unavailable,
    Slow(Duration),
}

/// How the archive mock answers on `GET /cdx/search/cdx`.
#[derive(Debug, Clone)]
pub struct Cdx {
    captures: Vec<Capture>,
    mode: CdxMode,
}

impl Cdx {
    /// Answers with real CDX semantics over `captures`.
    pub fn new(captures: Vec<Capture>) -> Self {
        Self {
            captures,
            mode: CdxMode::Serve,
        }
    }

    /// Answers 503 to everything.
    pub fn unavailable() -> Self {
        Self {
            captures: Vec::new(),
            mode: CdxMode::Unavailable,
        }
    }

    /// Like `new`, but waits `delay` before every reply.
    pub fn slow(captures: Vec<Capture>, delay: Duration) -> Self {
        Self {
            captures,
            mode: CdxMode::Slow(delay),
        }
    }

    fn rows(&self, params: &HashMap<String, String>) -> Vec<&Capture> {
        let day = |s: &str| s.chars().take(8).collect::<String>();
        let mut rows: Vec<&Capture> = self
            .captures
            .iter()
            .filter(|c| params.get("url").is_some_and(|u| *u == c.url))
            .filter(|c| {
                params
                    .get("filter")
                    .is_none_or(|f| f != "statuscode:200" || c.status == 200)
            })
            .filter(|c| {
                params
                    .get("to")
                    .is_none_or(|to| day(&c.timestamp) <= day(to))
            })
            .filter(|c| {
                params
                    .get("from")
                    .is_none_or(|from| day(&c.timestamp) >= day(from))
            })
            .collect();
        rows.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        match params.get("limit").and_then(|l| l.parse::<i64>().ok()) {
            Some(n) if n > 0 => rows.truncate(usize::try_from(n).unwrap_or(usize::MAX)),
            Some(n) if n < 0 => {
                let keep = usize::try_from(n.unsigned_abs()).unwrap_or(usize::MAX);
                let skip = rows.len().saturating_sub(keep);
                rows.drain(..skip);
            }
            _ => {}
        }
        rows
    }
}

fn json_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

struct CdxResponder {
    cdx: Arc<Mutex<Cdx>>,
    log: Log,
}

impl Respond for CdxResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.log.push(request);
        if request.url.path() != "/cdx/search/cdx" {
            return status(404);
        }
        let params: HashMap<String, String> = request
            .url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let cdx = self.cdx.lock().expect("cdx lock").clone();
        // Without `output=json` the real API answers plain text, which a JSON client can't use.
        if !matches!(cdx.mode, CdxMode::Unavailable)
            && params.get("output").map(String::as_str) != Some("json")
        {
            return status(400).set_body_string("only output=json is supported by this mock");
        }
        let rows = cdx.rows(&params);
        let body = if rows.is_empty() {
            "[]".to_owned()
        } else {
            let rows: Vec<String> = rows
                .iter()
                .map(|c| format!("[{},{}]", json_str(&c.timestamp), json_str(&c.original)))
                .collect();
            format!("[[\"timestamp\",\"original\"],{}]", rows.join(","))
        };
        let reply = match cdx.mode {
            CdxMode::Unavailable => return status(503),
            CdxMode::Serve => ResponseTemplate::new(200),
            CdxMode::Slow(delay) => ResponseTemplate::new(200).set_delay(delay),
        };
        reply
            .insert_header("content-type", "application/json")
            .set_body_string(body)
    }
}

// ---------------------------------------------------------------------------------------------
// Site directory and config
// ---------------------------------------------------------------------------------------------

/// The fixture's `rotguard.toml` settings; every field maps to one config key.
#[derive(Debug, Clone)]
pub struct Settings {
    pub host_interval_ms: u64,
    pub recheck_delay_ms: u64,
    pub archive_interval_ms: u64,
    pub timeout_ms: Option<u64>,
    pub deadline_s: Option<u64>,
    pub ignore: Vec<String>,
    /// Hosts configured with `soft404 = false`.
    pub no_soft404_hosts: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            host_interval_ms: 10,
            recheck_delay_ms: 50,
            archive_interval_ms: 10,
            timeout_ms: None,
            deadline_s: None,
            ignore: Vec::new(),
            no_soft404_hosts: Vec::new(),
        }
    }
}

impl Settings {
    fn render(&self, archive_url: &str) -> String {
        let mut toml = format!(
            "site_url = \"https://site.test\"\n\
             content = [\"*.md\"]\n\
             user_agent = \"{USER_AGENT}\"\n\
             archive_url = \"{archive_url}\"\n\
             host_interval_ms = {}\n\
             recheck_delay_ms = {}\n\
             archive_interval_ms = {}\n",
            self.host_interval_ms, self.recheck_delay_ms, self.archive_interval_ms
        );
        if let Some(ms) = self.timeout_ms {
            toml.push_str(&format!("timeout_ms = {ms}\n"));
        }
        if let Some(s) = self.deadline_s {
            toml.push_str(&format!("deadline_s = {s}\n"));
        }
        if !self.ignore.is_empty() {
            let items: Vec<String> = self.ignore.iter().map(|p| format!("'{p}'")).collect();
            toml.push_str(&format!("ignore = [{}]\n", items.join(", ")));
        }
        for host in &self.no_soft404_hosts {
            toml.push_str(&format!("[hosts.\"{host}\"]\nsoft404 = false\n"));
        }
        toml
    }
}

/// A temp site directory.
#[derive(Debug)]
pub struct Site {
    dir: TempDir,
}

impl Site {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temp dir"),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn write(&self, rel: &str, content: &str) {
        fs::write(self.dir.path().join(rel), content).expect("write fixture file");
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.dir.path().join(rel)).expect("read fixture file")
    }

    pub fn remove(&self, rel: &str) {
        fs::remove_file(self.dir.path().join(rel)).expect("remove fixture file");
    }

    /// Every file under the site directory with its bytes, keyed by relative path.
    pub fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(dir).expect("read dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    let rel = path.strip_prefix(root).expect("under root").to_owned();
                    out.insert(rel, fs::read(&path).expect("read file"));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(self.dir.path(), self.dir.path(), &mut out);
        out
    }
}

/// Writes `lines` into a file body at their 1-based line numbers; other lines are blank.
pub fn lines_at(lines: &[(usize, String)]) -> String {
    let last = lines.iter().map(|(n, _)| *n).max().unwrap_or(0);
    (1..=last)
        .map(|n| {
            lines
                .iter()
                .find(|(l, _)| *l == n)
                .map_or(String::new(), |(_, text)| text.clone())
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

// ---------------------------------------------------------------------------------------------
// The world: M, A and the site
// ---------------------------------------------------------------------------------------------

/// Mock servers `M` and `A` plus a site directory configured to use `A`.
pub struct World {
    m: MockServer,
    _a: MockServer,
    /// `M`'s origin, e.g. `http://127.0.0.1:<port>`.
    pub base: String,
    /// `A`'s origin.
    pub abase: String,
    /// Requests `M` received.
    pub log: Log,
    /// Requests `A` received.
    pub alog: Log,
    pub site: Site,
    settings: Settings,
    cdx: Arc<Mutex<Cdx>>,
}

impl World {
    /// `A` holds no captures until `archive` says otherwise.
    pub async fn new() -> Self {
        let m = MockServer::start().await;
        let a = MockServer::start().await;
        let alog = Log::default();
        let cdx = Arc::new(Mutex::new(Cdx::new(Vec::new())));
        Mock::given(any())
            .respond_with(CdxResponder {
                cdx: Arc::clone(&cdx),
                log: alog.clone(),
            })
            .mount(&a)
            .await;
        let world = Self {
            base: m.uri(),
            abase: a.uri(),
            m,
            _a: a,
            log: Log::default(),
            alog,
            site: Site::new(),
            settings: Settings::default(),
            cdx,
        };
        world.write_config();
        world
    }

    /// Replaces how `A` answers.
    pub fn archive(&self, cdx: Cdx) {
        *self.cdx.lock().expect("cdx lock") = cdx;
    }

    fn write_config(&self) {
        self.site
            .write("rotguard.toml", &self.settings.render(&self.abase));
    }

    /// Changes the fixture settings and rewrites `rotguard.toml`.
    pub fn configure(&mut self, change: impl FnOnce(&mut Settings)) {
        change(&mut self.settings);
        self.write_config();
    }

    /// Installs `M`'s routes. Call once.
    pub async fn serve(&self, routes: Routes) {
        Mock::given(any())
            .respond_with(Router {
                log: self.log.clone(),
                routes,
            })
            .mount(&self.m)
            .await;
    }

    /// An absolute URL on `M`.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// The snapshot URL `A` would propose for `path` on `M`.
    pub fn snapshot_url(&self, timestamp: &str, path: &str) -> String {
        format!("{}/web/{timestamp}/{}", self.abase, self.url(path))
    }

    /// Runs the binary on the site directory.
    pub async fn run(&self, args: &[&str]) -> Out {
        run(self.site.path(), args).await
    }
}

// ---------------------------------------------------------------------------------------------
// Running the binary and reading its output
// ---------------------------------------------------------------------------------------------

/// What one run of the binary produced.
#[derive(Debug)]
pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    /// Wall time of the child process.
    pub elapsed: Duration,
}

/// One report entry: header fields plus the optional detail line.
#[derive(Debug, Clone)]
pub struct Entry {
    /// `<file>:<line>`
    pub loc: String,
    /// `<CLASS> <statuses or reason>`
    pub verdict: String,
    pub url: String,
    /// The indented line after the header, trimmed.
    pub detail: Option<String>,
}

/// The counts on the summary line.
#[derive(Debug, Clone, Copy, Default)]
pub struct Summary {
    pub checked: usize,
    pub ok: usize,
    pub permanent: usize,
    pub temporary: usize,
    pub dead: usize,
    pub suspect: usize,
    pub unverifiable: usize,
    pub ignored: usize,
}

impl Summary {
    pub fn line(&self) -> String {
        format!(
            "checked {}: ok {}, permanent {}, temporary {}, dead {}, suspect {}, \
             unverifiable {}, ignored {}",
            self.checked,
            self.ok,
            self.permanent,
            self.temporary,
            self.dead,
            self.suspect,
            self.unverifiable,
            self.ignored
        )
    }
}

impl Out {
    /// The summary line (R19); fails the test if the run printed none.
    pub fn summary_line(&self) -> &str {
        let last = self.stdout.lines().rfind(|l| !l.trim().is_empty());
        match last {
            Some(line) if line.starts_with("checked ") => line,
            _ => panic!(
                "no summary line; exit {}\nstdout:\n{}\nstderr:\n{}",
                self.code, self.stdout, self.stderr
            ),
        }
    }

    pub fn assert_summary(&self, expected: Summary) {
        assert_eq!(
            self.summary_line(),
            expected.line(),
            "stdout:\n{}",
            self.stdout
        );
    }

    pub fn assert_code(&self, code: i32) {
        assert_eq!(
            self.code, code,
            "exit code; stdout:\n{}\nstderr:\n{}",
            self.stdout, self.stderr
        );
    }

    /// Every report entry in print order.
    pub fn entries(&self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = Vec::new();
        for line in self.stdout.lines() {
            if line.trim().is_empty() || line.starts_with("checked ") {
                continue;
            }
            if line.starts_with(' ') {
                let last = entries
                    .last_mut()
                    .unwrap_or_else(|| panic!("detail line before any header: {line:?}"));
                assert!(last.detail.is_none(), "two detail lines under {}", last.loc);
                last.detail = Some(line.trim().to_owned());
                continue;
            }
            let parts: Vec<&str> = line.splitn(3, "  ").collect();
            let [loc, verdict, url] = parts[..] else {
                panic!("malformed header line {line:?}");
            };
            entries.push(Entry {
                loc: loc.to_owned(),
                verdict: verdict.to_owned(),
                url: url.to_owned(),
                detail: None,
            });
        }
        entries
    }

    /// Entries whose original URL, without fragment, is `url`.
    pub fn entries_for(&self, url: &str) -> Vec<Entry> {
        self.entries()
            .into_iter()
            .filter(|e| e.url.split('#').next() == Some(url))
            .collect()
    }

    /// The only entry for `url`; fails the test otherwise.
    pub fn entry(&self, url: &str) -> Entry {
        let mut found = self.entries_for(url);
        assert_eq!(
            found.len(),
            1,
            "expected one entry for {url}; stdout:\n{}\nstderr:\n{}",
            self.stdout,
            self.stderr
        );
        found.remove(0)
    }
}

/// Runs the binary on `site_dir` with extra arguments.
pub async fn run(site_dir: &Path, args: &[&str]) -> Out {
    let dir = site_dir.to_owned();
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    tokio::task::spawn_blocking(move || {
        let start = Instant::now();
        let output = Command::new(env!("CARGO_BIN_EXE_rotguard"))
            .arg(&dir)
            .args(&args)
            .output()
            .expect("binary runs");
        Out {
            code: output.status.code().expect("exited normally"),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            elapsed: start.elapsed(),
        }
    })
    .await
    .expect("join")
}
