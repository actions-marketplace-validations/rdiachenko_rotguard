# Plan: rotguard, outbound link health for static sites
Design: docs/design/link-health.md

Order: skeleton, then the pending acceptance tests, then modules bottom-up (config, extract,
check, archive, report), then the wiring task that turns most scenarios on. The later tasks each
widen one behaviour and un-pend its scenarios.

Rules for every task:
- `src/lib.rs` declares each module as `pub mod`, and module APIs are `pub`, so a module
  that only its own tests use yet does not trip `dead_code` under `clippy -D warnings`.
- No dependency beyond the ones T1 adds.
- Only `src/apply.rs` writes files (fitness rule).
- Unit tests in `src/` use `wiremock` for HTTP and never touch the real network.

### T1: Crate skeleton
- Files: Cargo.toml, Cargo.lock, src/main.rs, src/lib.rs, tests/cli.rs
- Interfaces: Produces `pub struct Args { pub site_dir: PathBuf /* default "." */, pub apply: bool, pub strict: bool }`
  (clap derive: `rotguard [SITE_DIR] [--apply] [--strict]`) and
  `pub fn run(args: Args) -> std::process::ExitCode`, which returns `ExitCode::SUCCESS` for
  now. `main.rs` only parses `Args` and calls `run`. Package `rotguard` 0.1.0, edition 2024.
  Dependencies are the D10 crates: `tokio` (rt-multi-thread, macros, time, sync), `reqwest`
  (rustls-tls, no default features), `pulldown-cmark`, `toml`, a maintained YAML crate, `serde`
  (derive), `regex`, `globset`, `walkdir`, `clap` (derive), `chrono`, and `url` with its `serde`
  feature (so `Verdict` and `Hop`, which hold `Url`, can derive `Serialize`). Dev-only: `wiremock`
  and `tempfile`.
- Test: `tests/cli.rs` checks that `rotguard --version` exits 0 and prints `rotguard 0.1.0`, and
  that an unknown flag exits 2.
- Covers: none
- Commit: Create the rotguard crate skeleton
- Review: skip

### T2: Acceptance tests
- Files: tests/acceptance/main.rs, tests/acceptance/support.rs, tests/acceptance/config.rs,
  tests/acceptance/extract.rs, tests/acceptance/check.rs, tests/acceptance/soft404.rs,
  tests/acceptance/archive.rs, tests/acceptance/report.rs, tests/acceptance/apply.rs
- Interfaces: none. The tests drive only the binary
  (`std::process::Command::new(env!("CARGO_BIN_EXE_rotguard"))`) and the files in a temp site
  directory, so they compile before any module exists.
- Test: one test per scenario S1–S45, named after its S-ID (`s1_missing_config_exits_2`, …), each
  marked `#[ignore = "pending: S<n>"]`. `support.rs` holds the fixtures from the design's
  "Acceptance scenarios":
  - a `tempfile` site directory whose `rotguard.toml` sets `site_url = "https://site.test"`,
    `archive_url` = mock `A`, and small `host_interval_ms`, `recheck_delay_ms` and
    `archive_interval_ms`;
  - wiremock servers `M` and `A`, with a custom `Respond` that records each request's path,
    method, User-Agent and arrival `Instant` (S8, S9, S23, S30);
  - a helper that runs the binary and returns exit code, stdout and stderr.

  Run the tests on a multi-thread tokio runtime, with the binary as a child process.
  Links are written as inline markdown links `[x](…)`, except where a scenario names another
  form: front matter (S3, S4, S25–S27 dates, S35, S42), reference definitions, autolinks, raw
  HTML and bare text (S3, S39), code (S5), and the escaped link in S36. Bare-URL extraction
  arrives only in T12, after T11 un-pends most scenarios.
  Request assertions look only at the scenario's own paths. Probe requests, whose last segment
  is a 24-character `[a-z0-9]` token, are left out except in the probe scenarios S16, S17 and
  S21. Re-check and retry requests count only where a scenario counts them (S9, S22, S23, S29).
  Every test that expects a finished run also asserts that the summary line is printed (R19).
  That way no scenario passes before its feature exists, including "no request" and "no file
  changed" cases such as S5, S32 and S40.
  S31's suspect is a redirect to `/` (R10), so T13 can un-pend it.
  S32 compares file bytes before and after, with no hashing crate.
  S34's handler is a custom `Respond` that rewrites `post.md` before replying.
- Covers: S1, S2, S3, S4, S5, S6, S7, S8, S9, S10, S11, S12, S13, S14, S15, S16, S17, S18, S19, S20, S21, S22, S23, S24, S25, S26, S27, S28, S29, S30, S31, S32, S33, S34, S35, S36, S37, S38, S39, S40, S41, S42, S43, S44, S45
- Commit: Add acceptance tests for link health
- Review: skip

### T3: Load rotguard.toml
- Files: src/config.rs, src/lib.rs
- Interfaces: Produces `pub struct Config { site_url: Url, content: Vec<String>, date_key: String,
  user_agent: String, ignore: Vec<Regex>, not_found_phrases: Vec<String>, archive_url: Url,
  host_interval: Duration, archive_interval: Duration, recheck_delay: Duration, timeout: Duration,
  deadline: Duration, hosts: HashMap<String, HostConfig> }` (all fields `pub`), with
  `pub struct HostConfig { pub soft404: bool }` (default true), and
  `pub fn load(site_dir: &Path) -> Result<Config, ConfigError>`, a thin wrapper that reads
  `rotguard.toml` and calls `pub fn parse(text: &str) -> Result<Config, ConfigError>`. Defaults
  are as in the design's Config block. Unknown keys are an error. `ConfigError`'s `Display` names
  the missing file or the bad field. `run` loads the config first and, on error, prints it to
  stderr and exits 2.
- Test: unit tests call `parse` on strings, so no test writes a file under `src/` (the fitness
  rule allows writes only in `src/apply.rs`): a config with only `site_url` gets every default,
  an unknown key is rejected, and `site_url = 42` gives an error naming `site_url`. Un-pends S1, S2.
- Covers: R1, S1, S2
- Commit: Load and validate rotguard.toml

### T4: Extract front-matter links and page date
- Files: src/extract.rs, src/lib.rs
- Interfaces: Consumes `Config`. Produces
  `pub fn extract(path: &Path, text: &str, cfg: &Config) -> Page` with
  `pub struct Page { pub path: PathBuf, pub date: Option<NaiveDate>, pub links: Vec<Link> }`,
  `pub struct Link { pub url: String, pub line: u32, pub span: Range<usize>, pub ctx: Ctx, pub verbatim: bool }`,
  `pub enum Ctx { MdDest, MdText, Html, FrontMatter { quote: Quote } }` and
  `pub enum Quote { Single, Double, Bare }`. This task fills only front-matter links and `date`;
  it ignores the body. Only absolute http(s) URLs with a host are kept (R2). Spans follow the
  design's token rule: the whole string content between its own delimiters, matched against a
  parsed value; values written with escapes get `verbatim: false`.
- Test: unit tests on TOML (`+++`) and YAML (`---`) front matter:
  - nested `[[discussedOn]] url`;
  - a TOML datetime and a `YYYY-MM-DD…` string as the date;
  - `source = 'M/a'  # also 'M/a'` beside `other = 'M/ab'`, which yields exactly one span, on
    `source`'s value (the S42 rule);
  - an escaped value, which gets `verbatim: false`;
  - each span slicing back to the URL.
- Covers: R2
- Commit: Extract front-matter links and page date

### T5: Extract markdown body links
- Files: src/extract.rs
- Interfaces: Extends `extract` with body links. The body is parsed alone with
  `pulldown-cmark`'s `into_offset_iter`, and the body's start offset is added to every span and
  line. Covered: inline links and images (`MdDest`), reference definitions via
  `reference_definitions()` (one each), autolinks, and `href`/`src` in Html/InlineHtml events
  (`Html`). Reference-style uses, code blocks and inline code are skipped. A destination not
  found verbatim inside its event range keeps the event range as `span`, with `verbatim: false`.
  `mailto:`, relative, `#…` and `//host` destinations are skipped silently.
- Test: unit tests:
  - each link kind gives the right URL, line and file-absolute span;
  - fenced, indented and inline code yield nothing;
  - `[x](M/a\_b)` is `verbatim: false`;
  - a used and an unused reference definition give one link each;
  - non-http destinations yield nothing.
- Covers: R2, R3
- Commit: Extract markdown body links

### T6: Fetch URLs through per-host gates
- Files: src/check.rs, src/lib.rs
- Interfaces: Consumes `Config`. Produces
  `pub struct Hop { pub url: Url, pub status: u16 }`,
  `pub struct Fetched { pub chain: Vec<Hop>, pub body: Vec<u8>, pub retry_after: Option<Duration> }`,
  `pub enum FetchError { Dns, Timeout, Tls, Connect, NoLocation(Vec<Hop>), Loop(Vec<Hop>), TooManyHops(Vec<Hop>) }`
  and `pub struct Fetcher` with `pub fn new(cfg: &Config) -> Fetcher` and
  `pub async fn fetch(&self, url: &Url) -> Result<Fetched, FetchError>`.
  - Only GET, with the configured User-Agent and `timeout`.
  - Redirects are off (`Policy::none()`) and followed by hand, recording every hop; a loop or
    more than 10 hops is an error.
  - The body is read up to 256 KB.
  - Each host has a `Mutex<Instant>` gate held for the whole request: one request in flight per
    host, starts ≥ `host_interval` apart. A global semaphore of 8 caps hosts in flight.
- Test: wiremock unit tests:
  - a 301 → 302 → 200 chain is recorded hop by hop;
  - a self-redirect gives `Loop`;
  - 11 hops give `TooManyHops`;
  - a 300 KB body is cut to 256 KB;
  - three requests to one host start ≥ `host_interval` apart and carry the User-Agent;
  - a slow handler gives `Timeout`.
- Covers: R6
- Commit: Fetch URLs through per-host gates

### T7: Classify redirect chains
- Files: src/check.rs
- Interfaces: Produces `pub enum Verdict { Ok { chain }, Permanent { chain, fix: Url },
  Temporary { chain, final_url: Url }, Dead { reason: String }, Suspect { chain, reason: String },
  Unverifiable { reason: String } }` (derives `Serialize`) and
  `pub fn classify(outcome: &Result<Fetched, FetchError>) -> Verdict`. This is the design's
  classification order without the soft-404 steps (3–5) and without retries:
  1. final 404/410 or `Dns` → `Dead` ("404", "410", "dns");
  2. 401/403/429/999, 5xx, `Timeout`, `Tls`, `Connect` → `Unverifiable` with that reason;
  3. a leading 301/308 run ending in 2xx → `Permanent`, whose fix is the last 301/308 target;
  4. a first hop 302/303/307 ending in 2xx → `Temporary` with the final URL;
  5. 2xx with no redirect → `Ok`;
  6. anything else (other 4xx, `NoLocation`, `Loop`, `TooManyHops`) → `Unverifiable` with a
     reason naming it ("405", "redirect loop", …).
- Test: table-driven unit tests over hand-built chains, one per row above. Includes 308 → 302 →
  200 giving fix `p2` (the S13 rule) and 301 → 404 giving `Dead`.
- Covers: R7, R8, R9, R14
- Commit: Classify redirect chains

### T8: Check URLs with retries and deadline
- Files: src/check.rs
- Interfaces: Consumes `Fetcher`, `classify`. Produces
  `pub async fn check_all(urls: Vec<Url>, cfg: &Config, deadline: tokio::time::Instant) -> HashMap<Url, Verdict>`.
  - Timeouts, 5xx and connection errors are retried twice. A 429 is retried once, after its
    `Retry-After` (capped at 30 s).
  - A first `Dead` waits `recheck_delay` and fetches again; the second answer is the one
    classified.
  - Work for each URL stops at `deadline` (`tokio::time::timeout_at`), and unfinished URLs get
    `Unverifiable("deadline")`.
  - URLs on different hosts run concurrently, within the `Fetcher` gates.
- Test: wiremock unit tests:
  - 503 three times → `Unverifiable("503")` after 3 GETs;
  - a slow handler → `timeout` after 3 GETs;
  - 429 with `Retry-After: 1`, then 200 → `Ok`, with the GETs ≥ 1 s apart;
  - 404 then 200 → `Ok`;
  - 404 twice → `Dead`, with the GETs ≥ `recheck_delay` apart;
  - a hanging handler with a 1 s deadline → `Unverifiable("deadline")` within 2 s.
- Covers: R7, R14, R15
- Commit: Check URLs with retries and a deadline

### T9: Look up Wayback snapshots
- Files: src/archive.rs, src/lib.rs
- Interfaces: Consumes `Config`. Produces `pub struct Archive` with
  `pub fn new(cfg: &Config) -> Archive` and
  `pub async fn snapshot(&self, url: &Url, date: NaiveDate, deadline: tokio::time::Instant) -> Result<Url, NoFix>`,
  plus `pub enum NoFix { NoCapture, Unreachable, Deadline }`, whose `Display` is "no archived copy",
  "archive unreachable" and "deadline".
  - Two CDX queries go to `{archive_url}/cdx/search/cdx`, both with `url`,
    `filter=statuscode:200`, `fl=timestamp,original` and `output=json`. First
    `to=<YYYYMMDD>&limit=-1`; if that is empty, `from=<YYYYMMDD>&limit=1`.
  - All requests pass one shared gate (`archive_interval`), and each query gets 3 attempts.
  - The result is `{archive_url}/web/<timestamp>/<original>`, built from the capture's CDX
    `original` field rather than the queried URL (S35 relies on this). It has no fragment; `run`
    adds it.
- Test: wiremock unit tests:
  - a capture before the date wins over one after it (S25 rule);
  - only a later capture → that one (S26 rule);
  - empty → `NoCapture`;
  - 503 every time → `Unreachable` after exactly 3 requests;
  - three lookups start ≥ `archive_interval` apart;
  - a hanging archive with a 1 s deadline → `Deadline`.
- Covers: R15, R16, R17, R18
- Commit: Look up Wayback snapshots

### T10: Format the report
- Files: src/report.rs, src/lib.rs
- Interfaces: Consumes `Link`, `Verdict`. Produces in `lib.rs`
  `pub struct Finding { pub file: PathBuf /* relative to SITE_DIR */, pub link: Link, pub verdict: Verdict, pub fix: Fix }`
  and `pub enum Fix { None, Proposed(String), Manual(String), Applied(String) }`. In `report.rs`
  it produces `pub struct Summary { pub checked, ok, permanent, temporary, dead, suspect, unverifiable, ignored: usize }`
  and `pub fn render(findings: &[Finding], summary: &Summary) -> String`.
  - One entry per finding, sorted by file then line.
  - Header: `<file>:<line>  <CLASS> <statuses of the chain, space-separated, or reason>  <original URL>`.
    `CLASS` is one of `DEAD`, `PERMANENT REDIRECT`, `TEMPORARY REDIRECT`, `SUSPECT`,
    `UNVERIFIABLE`. `DEAD`, `SUSPECT` and `UNVERIFIABLE` print their reason (`SUSPECT redirect to
    root`); `PERMANENT REDIRECT` and `TEMPORARY REDIRECT` print the chain's statuses.
  - Detail line, indented 4 spaces:
    - dead, permanent and suspect: `fix: <url>` for `Proposed` and `Applied`, or
      `manual fix: <reason>`;
    - temporary: `final: <url>`;
    - unverifiable: none.
  - Last line: `checked N: ok …, permanent …, temporary …, dead …, suspect …, unverifiable …, ignored …`.
- Test: unit tests:
  - entries for `b.md:7`, `a.md:9`, `a.md:2` render in sorted order;
  - `PERMANENT REDIRECT 301 200` with `fix: …`;
  - `DEAD 404` with `manual fix: no archived copy`;
  - a temporary entry gets `final:` and no `fix:`;
  - an unverifiable entry has no detail line;
  - a suspect entry prints `SUSPECT <reason>` with its `fix:` or `manual fix:` line;
  - the summary line matches the design's example.
- Covers: R19
- Commit: Format the findings report

### T11: Wire the main flow
- Files: src/lib.rs
- Interfaces: Consumes `config::load`, `extract`, `check_all`, `Archive::snapshot`, `report::render`.
  `run` performs main-flow steps 1–6 and 8–9. `--apply` is parsed but not acted on until T17,
  so no file is written (R20).
  - Glob `content` under SITE_DIR with `globset` + `walkdir`. A file that can't be read prints
    the reason to stderr and exits 2.
  - Drop URLs whose host is `site_url`'s host. Drop those matching an `ignore` regex, counting
    the distinct dropped URLs as `ignored`.
  - Dedupe the rest without their fragment, then set `deadline = now + deadline_s` before
    calling `check_all`.
  - For each dead or suspect occurrence, look up `snapshot` per (URL, page date, or the run date
    when the page has none).
  - Build one `Finding` per non-ok occurrence:
    - a permanent fix gets the link's fragment when the target has none (R8);
    - a snapshot always gets the link's fragment (R16);
    - `NoFix` becomes `Fix::Manual`.
  - Print `render` to stdout.
  - Exit 1 if any dead, permanent or suspect finding is not `Applied`, or, with `--strict`, any
    temporary or unverifiable finding. Otherwise exit 0.
- Test: un-pends S4, S5, S6, S7, S8, S9, S10, S11, S12, S13, S14, S22, S23, S24, S25, S26, S27,
  S28, S29, S30, S32, S38, S40, S41, S44.
- Covers: R4, R5, R15, R16, R20, R24, S4, S5, S6, S7, S8, S9, S10, S11, S12, S13, S14, S22, S23, S24, S25, S26, S27, S28, S29, S30, S32, S38, S40, S41, S44
- Commit: Wire the link check main flow

### T12: Extract bare URLs
- Files: src/extract.rs
- Interfaces: Extends `extract` with `MdText` links. For each paragraph or other inline
  container, a regex scans its source slice, skipping sub-ranges covered by code, link
  destinations and HTML events.
  - A URL ends at whitespace, `"`, `'`, `<` or `>`.
  - Trailing `.,;:!?` is then trimmed, and a trailing `)` is trimmed unless its parens balance.
  - Shortcode arguments such as `src="…"` are covered too.
- Test: unit tests:
  - `see M/a_b_(c)&x=1 now` gives exactly `M/a_b_(c)&x=1`;
  - `(see M/x).` gives `M/x`;
  - `{{< figure src="M/s" alt="x" >}}` gives `M/s`;
  - a URL already inside `[x](…)` is not duplicated;
  - a URL inside inline code is skipped.

  Un-pends S3, S39.
- Covers: R2, S3, S39
- Commit: Extract bare URLs from text

### T13: Flag redirects to the site root
- Files: src/check.rs
- Interfaces: `classify` gains the soft-404 step 3 and takes the host's config:
  `pub fn classify(outcome: &Result<Fetched, FetchError>, host: &HostConfig) -> Verdict`, with
  `check_all` updated to pass it. A URL whose path is not `/`, redirected to a URL whose path is
  `/`, becomes `Suspect("redirect to root")`, before the redirect-kind step, unless
  `soft404 = false` for that host.
- Test: unit tests:
  - 301 → `/` gives `Suspect`;
  - 302 → `/` → 200 gives `Suspect`, not `Temporary`;
  - the same chain with `soft404 = false` gives `Permanent`;
  - a root URL redirected to `/` is unaffected.

  Un-pends S15, S43, S31.
- Covers: R10, R13, S15, S43, S31
- Commit: Flag redirects to the site root

### T14: Probe for soft-404 pages
- Files: src/check.rs
- Interfaces: Inside `check_all`, a 2xx on a non-root path (soft404 on) fetches the probe: same
  scheme, host and parent path of the checked URL, with the last segment replaced by a random
  24-character `[a-z0-9]` token from std (no new crate). The probe goes through the same
  `Fetcher` gates and is cached per (host, parent path), storing final URL, status, `<title>` and
  word-3-shingle set. The URL becomes `Suspect` if the probe also ends in 2xx and one of these
  holds:
  - same final URL;
  - same non-empty title;
  - Jaccard similarity ≥ 0.9 between the visible text of the first 256 KB of each body.

  Title and visible text come from regexes over the body.
- Test: unit tests:
  - shingle Jaccard of identical text is 1.0, and of unrelated text is < 0.9;
  - title extraction works;
  - two URLs under `/docs/` with a 404 probe → both ok, one probe request;
  - same title → `Suspect`;
  - same body, different title → `Suspect` naming body similarity;
  - `soft404 = false` → no probe.

  Un-pends S16, S17, S18, S21.
- Covers: R11, R13, S16, S17, S18, S21
- Commit: Probe for soft-404 pages

### T15: Flag not-found titles
- Files: src/check.rs
- Interfaces: After the probe step, a 2xx whose probe did not answer 404 or 410 becomes `Suspect`
  naming the phrase, if its title contains a `not_found_phrases` entry, case-insensitively.
  Skipped when the host's `HostConfig` has `soft404 = false` (R13), like the steps from T13–T14.
- Test: unit tests:
  - "Page Not Found – Example" with a 200 probe → `Suspect` naming "not found";
  - "404 errors explained" with a 404 probe → ok;
  - "Page Not Found – Example" with a 200 probe on a host with `soft404 = false` → ok.

  Un-pends S19, S20.
- Covers: R12, R13, S19, S20
- Commit: Flag pages with not-found titles

### T16: Decide which fixes can be written
- Files: src/apply.rs, src/lib.rs
- Interfaces: Consumes `Link`, `Ctx`. Produces
  `pub fn writable(fix: &str, link: &Link) -> Result<(), String>`. The `Err` reason is
  "not verbatim" for `verbatim: false`, and "cannot quote" when:
  - the fix has `'` in a `FrontMatter { quote: Single }` value;
  - the fix has `"` in a `FrontMatter { quote: Double }` value;
  - in `MdDest`, the fix has whitespace, unbalanced `(`/`)`, `<` or `>`.
- Test: table-driven unit tests, one row per rule plus an accepted fix for each `Ctx`.
- Covers: R23
- Commit: Decide which fixes can be written

### T17: Apply fixes with --apply
- Files: src/apply.rs, src/lib.rs
- Interfaces: Consumes `Finding`, `writable`. Produces
  `pub fn apply(site_dir: &Path, findings: &mut [Finding]) -> std::io::Result<()>`. `run` calls it
  when `--apply` is given, before printing. Only `Proposed` fixes on `Permanent` and `Dead` are
  applied; suspects never are. Per file:
  1. Findings failing `writable` become `Manual(reason)` and leave the edit list (R23).
  2. The file is re-read. If any remaining span no longer holds its original URL, the whole
     file is left untouched and its findings become `Manual("changed during run")` (R22).
  3. Otherwise the spans are replaced from last to first and written to `<file>.rotguard.tmp` in
     the same directory, then renamed over the original. Those findings become
     `Applied(fix)`, which no longer counts as unfixed for the exit code (R24).
- Test: unit tests on a tempdir file:
  - two spans replaced, every other byte unchanged;
  - a changed span leaves the file byte-identical;
  - a non-verbatim finding is set aside while the other fix still applies;
  - no `.rotguard.tmp` remains.

  Un-pends S33, S34, S35, S36, S37, S42, S45.
- Covers: R21, R22, R23, R24, S33, S34, S35, S36, S37, S42, S45
- Commit: Apply fixes to sources with --apply

## Verification contract
```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'
! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'
scripts/privacy-check.sh --all
```
