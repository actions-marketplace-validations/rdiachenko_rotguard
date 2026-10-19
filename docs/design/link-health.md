---
status: approved
---
# rotguard: outbound link health for static sites

## Problem

Static sites accumulate outbound links that rot: pages move, sites die, and some hosts answer a
missing page with a friendly "200 OK, not found" (a soft-404) that ordinary link checkers count as
healthy. rdiachenko.com has about 87 distinct external URLs across 37 hosts in 38 markdown files
and no link checking at all today, so rot is found only when a reader hits it. Fixing rot by hand
is tedious: find the link in the source, work out where it went, or dig up an archived copy from
around the time the post was written. rotguard finds dead links, redirects and soft-404s in a
site's markdown sources and proposes, and on request applies, a fix for each. It is configured per
site so it works for any markdown-based static site, not only this one.

## Scope

In:
- A Rust CLI run locally against a site directory, configured by `rotguard.toml` in that directory.
- Link discovery in markdown sources:
  - inline links and images;
  - reference definitions and autolinks;
  - bare URLs in text, including inside shortcode arguments;
  - `href`/`src` in raw HTML;
  - every http(s) string value in TOML (`+++`) or YAML (`---`) front matter.
- Checking: dead links, permanent and temporary redirects, soft-404 suspects, unverifiable links.
- Fixes:
  - permanent redirects are rewritten to the target of their last leading 301/308 hop;
  - dead links get the Wayback Machine snapshot closest to the page date.
- A dry-run report by default. `--apply` edits the source files in place.
- Exit codes suitable for a CI job.

Out:
- Internal links (links to the site's own host) and anchors within the site.
- Scanning built HTML, and links that exist only in templates or theme files.
- State between runs: no cache and no "dead after N runs" (run-state file parked in
  `docs/backlog.md`).
- A scheduled CI workflow or bot that opens issues or PRs (weekly GitHub Action parked in
  `docs/backlog.md`).
- Consulting `robots.txt`.
- Interactive prompting for replacements.
- Saving pages to the Wayback Machine ("save page now").
- JSON or other machine-readable report formats.
- A `rotguard init` command to generate config.

## Requirements

- R1: WHEN rotguard runs on a directory THE SYSTEM SHALL load `rotguard.toml` from that directory, and IF the file is missing or invalid THEN it SHALL print the reason and exit with code 2 without making any request.
- R2: THE SYSTEM SHALL extract absolute `http://` or `https://` URLs that have a host from these places in files matching the configured content globs:
  - inline links and images;
  - reference definitions (one per definition, not per use) and autolinks;
  - bare URLs in text, including inside shortcode arguments;
  - `href` and `src` attribute values in raw HTML;
  - every string value at any depth of the front matter.
  Each URL is recorded with its file, line and the file-absolute byte span of the URL in the source. Every other destination (`mailto:`, relative paths, `#fragment`, protocol-relative `//host`) is skipped silently.
- R3: THE SYSTEM SHALL NOT extract URLs that appear inside fenced code blocks, indented code blocks or inline code.
- R4: THE SYSTEM SHALL check only external URLs: extracted URLs whose host differs from the host of the configured `site_url` and that match none of the configured `ignore` regexes. It SHALL report only the number of URLs dropped by `ignore`. Internal and skipped destinations are not counted.
- R5: THE SYSTEM SHALL check each distinct URL (compared without its fragment) at most once per run, however many times it appears. Soft-404 probes and the dead re-check are separate requests and don't count against this.
- R6: THE SYSTEM SHALL send only GET requests carrying the configured User-Agent. It SHALL keep at most one request in flight per host, leave at least `host_interval_ms` (default 1000) between the start of consecutive requests to the same host, and keep at most 8 hosts in flight at once.
- R7: WHEN a URL's final status, after following redirects, is 404 or 410, or its host does not resolve, on two attempts at least `recheck_delay_ms` (default 5000) apart THE SYSTEM SHALL classify it **dead**. A chain such as 301 → 404 is dead.
- R8: WHEN a URL's redirect chain starts with one or more 301/308 hops and ends in a 2xx response that is not a soft-404 suspect THE SYSTEM SHALL classify it **permanent redirect**. The proposed fix is the target of the last 301/308 in that leading run. If the target has no fragment, the original fragment is appended.
- R9: WHEN a URL's first redirect hop is 302, 303 or 307 and the chain ends in a 2xx response that is not a soft-404 suspect THE SYSTEM SHALL classify it **temporary redirect**, report the final URL, and propose no fix.
- R10: WHEN a URL whose path is not `/` is redirected to a URL whose path is `/` THE SYSTEM SHALL classify it **soft-404 suspect**.
- R11: WHEN a URL with a non-root path ends in a 2xx response THE SYSTEM SHALL fetch a probe URL. The probe is the same scheme, host and parent path, with the last path segment replaced by a random 24-character lowercase alphanumeric token. Each (host, parent path) is probed once per run. The URL is classified **soft-404 suspect** if the probe also ends in 2xx and at least one of these holds:
  - both have the same final URL;
  - both have the same non-empty `<title>`;
  - the visible text of the first 256 KB of each body has word-3-shingle Jaccard similarity ≥ 0.9.

  Comparing only the first 256 KB is deliberate: it bounds memory and time, at the cost of rare false suspects on very large pages whose openings match.
- R12: WHEN a URL ends in 2xx and its probe did not answer 404 or 410 THE SYSTEM SHALL also classify the URL **soft-404 suspect** if its `<title>` contains, case-insensitively, any configured not-found phrase. The defaults are "not found", "404", "page does not exist" and "page doesn't exist".
- R13: WHERE the config sets `soft404 = false` for a host THE SYSTEM SHALL skip R10–R12 for URLs on that host.
- R14: IF a URL answers 401, 403, 429 or 999, a 5xx, a timeout (`timeout_ms`, default 10000) or a TLS or connection error THEN THE SYSTEM SHALL classify it **unverifiable** with that reason. Before doing so it retries in two cases:
  - timeouts, 5xx and connection errors: two retries;
  - 429: one retry, after its `Retry-After` delay capped at 30 s.

  A 2xx reached without any redirect that R11–R13 do not mark as a suspect is **ok**. Any other outcome that R7–R13 do not classify is also **unverifiable**, with a reason naming it. That covers other 4xx statuses (e.g. 400, 405, 451), 3xx without `Location`, redirect loops, chains of more than 10 hops, and chains ending in 5xx.
- R15: IF the run's overall deadline (`deadline_s`, default 300, covering checking and archive lookups) passes THEN THE SYSTEM SHALL classify every unfinished URL as **unverifiable** (reason "deadline"), report every unfinished archive lookup as "manual fix: deadline", and finish the report.
- R16: WHEN a URL is dead or a soft-404 suspect THE SYSTEM SHALL ask the Wayback CDX API for status-200 captures of that URL. It proposes the latest capture on or before the page date, or, if there is none, the earliest capture after it. Pages without a date use the run date. The proposal is `<archive_url>/web/<timestamp>/<original>`, followed by the link's fragment if it has one. Lookups are made per (URL, page date).
- R17: IF the archive has no status-200 capture, or does not answer after 3 attempts THEN THE SYSTEM SHALL report the finding as "manual fix" with the reason.
- R18: THE SYSTEM SHALL leave at least `archive_interval_ms` (default 4000) between the start of consecutive archive requests.
- R19: WHEN a run finishes THE SYSTEM SHALL print one entry per occurrence of a non-ok URL, sorted by file then line. Each entry has a header line `<file>:<line>  <CLASS> <status chain or reason>  <original URL>`, followed by one indented detail line that depends on the class:
  - dead, permanent redirect and soft-404 suspect: `fix: <url>` or `manual fix: <reason>`;
  - temporary redirect: `final: <url>`;
  - unverifiable: no detail line (the reason is in the header).

  A last summary line counts distinct URLs: checked, ok, permanent redirect, temporary redirect, dead, suspect, unverifiable and ignored.
- R20: WHILE `--apply` is not given THE SYSTEM SHALL NOT modify, create or delete any file in the site directory.
- R21: WHEN `--apply` is given THE SYSTEM SHALL replace the bytes at each finding's span with its proposed fix, for permanent redirects and for dead links that have a snapshot only. It SHALL change no other byte of the file and SHALL write each file atomically (temp file plus rename).
- R22: IF the bytes at any span about to be applied in a file no longer equal the original URL THEN THE SYSTEM SHALL leave that whole file unchanged and report it as "changed during run". Findings that R23 turns into "manual fix" are set aside before this check and never make a file count as changed.
- R23: IF the link was extracted with `verbatim: false`, or the fix cannot be written in that position THEN THE SYSTEM SHALL report "manual fix" and not apply it. The fix cannot be written when:
  - it contains `'` inside a single-quoted TOML or YAML string;
  - it contains `"` inside a double-quoted string;
  - it contains whitespace, an unbalanced `(` or `)`, or `<` or `>` in a markdown link
    destination (`Ctx::MdDest`: inline, reference definition or autolink).
- R24: THE SYSTEM SHALL exit with code 0 when no finding remains unfixed, 1 when any remains unfixed, and 2 on a tool error. Unfixed findings are dead, permanent redirect and soft-404 suspect, plus unverifiable and temporary redirect when `--strict` is given.

## Acceptance scenarios

Fixtures:
- A temp site directory with `site_url = "https://site.test"`.
- A local mock HTTP server `M` (e.g. `http://127.0.0.1:<port>`).
- `archive_url` pointing at a mock archive `A`.
- `host_interval_ms`, `recheck_delay_ms` and `archive_interval_ms` set small unless the scenario says otherwise.

- S1 [R1]: Given a directory without `rotguard.toml`, when rotguard runs, then it exits 2, stderr names the missing file, and `M` receives no request.
- S2 [R1]: Given a `rotguard.toml` with `site_url = 42`, when rotguard runs, then it exits 2 and stderr names `site_url`.
- S3 [R2]: Given `post.md` with:
  - TOML front matter `source = 'M/a'` and a `[[discussedOn]]` table with `url = 'M/b'`;
  - a body with `[x](M/c)`, `![i](M/d)`, `[r]: M/e`, `<M/f>`, a bare `see M/g today`, and `<a href="M/h">`,
  when rotguard runs, then `M` receives one GET for each of `/a` to `/h`, and every finding line names `post.md` with the correct line number.
- S4 [R2]: Given `post.md` with YAML front matter `link: "M/y"`, when rotguard runs, then `M` receives a GET for `/y`.
- S5 [R3]: Given a body with `M/k1` in a fenced code block, `M/k2` in an indented code block and `` `M/k3` `` inline, when rotguard runs, then `M` receives no request for `/k1`, `/k2` or `/k3`.
- S6 [R4]: Given links `https://site.test/about`, `M/skip/1` with `ignore = ["/skip/"]`, and `M/ok`, when rotguard runs, then `M` receives a request only for `/ok`, `site.test` is never contacted, and the summary shows `ignored 1`.
- S7 [R5]: Given `M/dup` linked from three files, once as `M/dup#x`, when `/dup` answers 200 and a probe answers 404, then `M` receives exactly one GET for `/dup`.
- S8 [R6]: Given `host_interval_ms = 300` and three links to different paths on `M`, when rotguard runs, then the mock's recorded start times of consecutive requests are ≥ 300 ms apart, no two requests overlap, every request is GET, and each carries the configured User-Agent.
- S9 [R7]: Given `M/gone` answering 404 both times and `recheck_delay_ms = 200`, when rotguard runs, then `M` records two GETs for `/gone` ≥ 200 ms apart and the report classifies it `DEAD`.
- S10 [R7]: Given `M/flaky` answering 404 then 200 (and its probe 404), when rotguard runs, then it is classified ok and exit code is 0.
- S11 [R7]: Given a link `https://nonexistent.invalid/page`, when rotguard runs, then it is classified `DEAD` with reason containing `dns`.
- S12 [R8]: Given `M/old#sec` → 301 to `M/new` → 200 (probe 404), when rotguard runs, then the entry reads `PERMANENT REDIRECT 301 200` with `fix: M/new#sec`.
- S13 [R8]: Given `M/p1` → 308 → `M/p2` → 302 → `M/p3` → 200, when rotguard runs, then the fix is `M/p2`.
- S14 [R9, R19]: Given `M/t1` → 302 → `M/t2` → 200, when rotguard runs, then the entry reads `TEMPORARY REDIRECT` with the detail line `final: M/t2`, no `fix:` line, and exit code is 0. With `--strict` the exit code is 1.
- S15 [R10]: Given `M/deep/page` → 301 → `M/`, when rotguard runs, then it is classified `SUSPECT` with reason `redirect to root` and no `PERMANENT REDIRECT` fix.
- S43 [R9, R10]: Given `M/deep/tmp` → 302 → `M/` → 200, when rotguard runs, then it is classified `SUSPECT` (not `TEMPORARY REDIRECT`) and the exit code is 1.
- S16 [R11]: Given `M/docs/real` returns 200 with a `<title>` of "Docs home", and any `M/docs/<token>` returns 200 with that same title, when rotguard runs, then `/docs/real` is `SUSPECT` and the probe path matches `^/docs/[a-z0-9]{24}$`.
- S17 [R11]: Given `M/docs/a` and `M/docs/b` return 200 and `M/docs/<token>` returns 404, when rotguard runs, then both are ok and `M` received exactly one probe under `/docs/`.
- S18 [R11]: Given `M/x/a` returns 200 with a 400-word body and `M/x/<token>` returns 200 with the same body but a different title, when rotguard runs, then `/x/a` is `SUSPECT` with reason naming body similarity.
- S19 [R12]: Given `M/z/a` returns 200 with title "Page Not Found – Example" and `M/z/<token>` returns 200 with an unrelated title and body, when rotguard runs, then `/z/a` is `SUSPECT` with reason naming the title phrase.
- S20 [R12]: Given `M/w/a` returns 200 with title "404 errors explained" and `M/w/<token>` returns 404, when rotguard runs, then `/w/a` is ok.
- S21 [R13]: Given the S16 setup plus `[hosts."127.0.0.1"] soft404 = false`, when rotguard runs, then `/docs/real` is ok and `M` received no probe request.
- S22 [R14]: Given `M/forbidden` → 403, `M/err` → 503 three times, and `M/slow` sleeping longer than `timeout_ms = 200`, when rotguard runs, then all three are `UNVERIFIABLE` with reasons `403`, `503` and `timeout`. `M` records 1, 3 and 3 GETs respectively, and the exit code is 0.
- S23 [R14]: Given `M/busy` → 429 with `Retry-After: 1` then 200 (probe 404), when rotguard runs, then the two GETs are ≥ 1 s apart and `/busy` is ok.
- S24 [R15]: Given `deadline_s = 1` and `M/hang` that never responds within 5 s with `timeout_ms = 10000`, when rotguard runs, then it finishes within 3 s, `/hang` is `UNVERIFIABLE` with reason `deadline`, and the summary line is printed.
- S44 [R15]: Given `deadline_s = 2`, a dead `M/gone` with `recheck_delay_ms = 200`, and `A` that sleeps 10 s before every reply, when rotguard runs, then it finishes within 4 s and the `/gone` entry reads `manual fix: deadline`.
- S25 [R16]: Given a dead `M/gone` on a page dated `2021-03-04`, and `A` holding status-200 captures at `20200101000000` and `20220101000000`, when rotguard runs, then the fix is `A/web/20200101000000/M/gone`.
- S26 [R16]: Given the S25 setup with the only capture at `20220101000000`, when rotguard runs, then the fix is `A/web/20220101000000/M/gone`.
- S27 [R16]: Given a dead `M/gone#part` linked from pages dated 2019-05-01 and 2023-05-01, with captures at 2019-01-01 and 2023-01-01, when rotguard runs, then the two occurrences get different snapshot fixes, each ending in `#part`.
- S28 [R17]: Given a dead `M/gone` and `A` answering with no captures, when rotguard runs, then the entry reads `manual fix: no archived copy`.
- S29 [R17]: Given a dead `M/gone` and `A` answering 503 every time, when rotguard runs, then `A` received exactly 3 requests and the entry reads `manual fix: archive unreachable`.
- S30 [R18]: Given `archive_interval_ms = 300` and three dead URLs, when rotguard runs, then consecutive archive requests start ≥ 300 ms apart.
- S31 [R19]: Given findings in `b.md:7`, `a.md:9` and `a.md:2` and one ok link, when rotguard runs, then the entries appear in the order `a.md:2`, `a.md:9`, `b.md:7`. The last line is `checked 4: ok 1, permanent 1, temporary 0, dead 1, suspect 1, unverifiable 0, ignored 0`, matching each scenario's verdicts.
- S32 [R20]: Given the S12 and S25 setups without `--apply`, when rotguard runs, then the SHA-256 of every file in the site directory is unchanged and no file is added or removed.
- S33 [R21]: Given `post.md` containing a permanent redirect `M/old`, a dead link with a snapshot, a suspect and a temporary redirect, when rotguard runs with `--apply`:
  - `post.md` differs from the original only in the two replaced URL spans;
  - the suspect and the temporary redirect URLs are byte-identical;
  - the TOML front matter still parses;
  - no temp file remains in the directory.
- S34 [R22]: Given a mock where `M/old`'s 301 handler rewrites `post.md`'s `M/old` to `M/other` before replying, when rotguard runs with `--apply`, then `post.md` equals what the handler wrote and the report says `changed during run`.
- S35 [R23]: Given front matter `source = 'M/gone'` and a snapshot whose original URL contains `'`, when rotguard runs with `--apply`, then the line is unchanged and the report says `manual fix: cannot quote`.
- S36 [R23]: Given the link `[x](M/a\_b)`, which source-escapes `_`, redirecting 301 to `M/c`, when rotguard runs with `--apply`, then the line is unchanged and the report says `manual fix`. A second link in the same file, `M/old` → 301 → `M/new`, is still rewritten (no `changed during run`).
- S45 [R23]: Given the link `[x](M/p)` redirecting 301 to `M/a)b` → 200, when rotguard runs with `--apply`, then the line is unchanged and the report says `manual fix: cannot quote`.
- S37 [R24]: Given only ok links, rotguard exits 0. Given one dead link without `--apply`, it exits 1. Given one permanent redirect with `--apply`, it applies the fix and exits 0. Given an unreadable content file, it exits 2.
- S38 [R24]: Given one unverifiable link and nothing else, rotguard exits 0, and exits 1 with `--strict`.
- S39 [R2]: Given a body line `see M/a_b_(c)&x=1 now` and a line `{{< figure src="M/s" alt="x" >}}`, when rotguard runs, then `M` receives one GET for exactly `/a_b_(c)&x=1` and one for `/s`.
- S40 [R2, R4]: Given links `[m](mailto:a@example.com)`, `[r](/posts/x/)`, `[f](#top)` and `[p](//127.0.0.1/p)`, and no `ignore`, when rotguard runs, then `M` receives no request and the summary shows `checked 0` and `ignored 0`.
- S41 [R7, R14]: Given `M/moved` → 301 → `M/gone2` → 404 (both attempts), `M/bad` → 405, and `M/loop` → 301 → `M/loop`, when rotguard runs, then `/moved` is `DEAD`, `/bad` is `UNVERIFIABLE` with reason `405`, and `/loop` is `UNVERIFIABLE` with reason `redirect loop`.
- S42 [R2]: Given front matter `source = 'M/a'  # also 'M/a'` and `other = 'M/ab'`, with `M/a` → 301 → `M/new` → 200, when rotguard runs with `--apply`, then only the value of `source` changes to `'M/new'`; the comment and `other` are byte-identical.

## Design

One binary crate, `rotguard`, with a library inside (`src/lib.rs`) so integration tests drive it
in-process and via the binary. `lib.rs::run(args) -> ExitCode` performs the main flow; `main.rs`
only parses arguments and calls it.

```
rotguard [SITE_DIR=.] [--apply] [--strict]
```

```text
Context: rotguard (dashed box = system boundary)

                            ┌────────────────┐
                            │   Site owner   │
                            └──┬─────────▲───┘
 rotguard [DIR] [--apply]      │         │ report on stdout, errors on stderr,
                [--strict]     │         │ exit code 0 / 1 / 2
                            ┌╌╌▼╌╌╌╌╌╌╌╌╌┴╌╌╌┐ GET URL, follow redirects  ┌──────────────────┐
┌──────────────────┐ read   ╎                ╎───────────────────────────►│ Linked websites  │
│ Site directory   ├───────►╎    rotguard    ╎ (HTTP(S), ≤1 req/host/s)   └──────────────────┘
│  rotguard.toml   │        ╎   [Rust CLI]   ╎
│  content/**/*.md │◄───────╎                ╎ CDX query: closest capture ┌──────────────────┐
└──────────────────┘ write  ╎                ╎───────────────────────────►│ Wayback Machine  │
                     spans  └╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┘ (HTTPS, JSON, 1 req/4 s)   └──────────────────┘
                  (--apply only)
```

```text
Components: one crate; lib.rs::run calls each module in turn, and modules never call each other

┌─────────────────────┐ SITE_DIR                    ┌────────────────────────────────────┐
│ lib.rs::run         ├────────────────────────────►│ config.rs   parse rotguard.toml    │
│ main flow 1–9:      │◄────────────────────────────┤ serde; unknown keys are errors     │
│ globs content files,│ Config, or error → exit 2   └────────────────────────────────────┘
│ dedupes Urls,       │ path, text of each file     ┌────────────────────────────────────┐
│ builds Findings,    ├────────────────────────────►│ extract.rs  front matter + body    │
│ picks the exit code │◄────────────────────────────┤ → Links with file-absolute spans   │
│                     │ Page{path, date, links}     └────────────────────────────────────┘
│                     │ distinct Urls (no fragment) ┌────────────────────────────────────┐
│                     ├────────────────────────────►│ check.rs    fetch, redirect, probe │
│                     │◄────────────────────────────┤ host gates, deadline → Verdict     │
│                     │ HashMap<Url, Verdict>       └────────────────────────────────────┘
│                     │ (Url, date) if dead/suspect ┌────────────────────────────────────┐
│                     ├────────────────────────────►│ archive.rs  Wayback CDX lookup     │
│                     │◄────────────────────────────┤ shared archive gate, 3 tries       │
│                     │ Result<Url, NoFix>          └────────────────────────────────────┘
│                     │ Findings, with --apply only ┌────────────────────────────────────┐
│                     ├────────────────────────────►│ apply.rs    verify spans, rewrite  │
│                     │◄────────────────────────────┤ tmp + rename; the only writer      │
│                     │ per file: outcome           └────────────────────────────────────┘
│                     │ Findings, summary counts    ┌────────────────────────────────────┐
│                     ├────────────────────────────►│ report.rs   plain text to stdout   │
│                     │                             │ sorted by file, then line          │
└─────────────────────┘                             └────────────────────────────────────┘
```

**Config** (`src/config.rs`): parsed from `SITE_DIR/rotguard.toml` with serde; unknown keys are an error.

```toml
site_url = "https://rdiachenko.com"           # required; its host = internal
content = ["content/**/*.md"]                 # default
date_key = "date"                             # default; front matter key holding the page date
user_agent = "rotguard/0.1 (+https://rdiachenko.com)"  # default "rotguard/<version>"
ignore = []                                   # URL regexes
not_found_phrases = ["not found", "404", "page does not exist", "page doesn't exist"]
archive_url = "https://web.archive.org"
host_interval_ms = 1000
archive_interval_ms = 4000
recheck_delay_ms = 5000
timeout_ms = 10000
deadline_s = 300
[hosts."goodreads.com"]
soft404 = false
```

**Extract** (`src/extract.rs`): `fn extract(path, text: &str, cfg) -> Page`.

```rust
struct Page { path: PathBuf, date: Option<NaiveDate>, links: Vec<Link> }
struct Link { url: String, line: u32, span: Range<usize>, ctx: Ctx, verbatim: bool }
enum Ctx { MdDest, MdText, Html, FrontMatter { quote: Quote } }  // Quote: Single | Double | Bare
```

- Front matter is the leading `+++…+++` (TOML) or `---…---` (YAML) block. It is parsed with the
  `toml` crate or a YAML crate, collecting every http(s) string value and the `date_key` value. A
  TOML datetime or a string whose first 10 chars are `YYYY-MM-DD` both count as the date.
- Front-matter spans are recorded only for the whole content of a string value between its own
  delimiters: `'…'` or `"…"`, or a YAML plain scalar running to the end of the line. A lexer
  over the block finds those string tokens, and a token counts only if its content equals a
  parsed value. Comments, keys and prefixes of longer values never match (S42). The quote is the
  delimiter. A value written with escapes doesn't appear verbatim, so it gets `verbatim: false`.
- The body is the text after the closing front-matter delimiter. It is parsed on its own with
  `pulldown-cmark` `into_offset_iter`, and the body's start offset is added to every span and line,
  so spans are file-absolute and front matter is never read as markdown.
  - Inline Link and Image events give the destination; its span is found verbatim inside the
    event range. Link events whose `link_type` is a reference (`Reference`, `Collapsed`,
    `Shortcut` and their `Unknown` variants) are skipped.
  - Reference definitions come from `Parser::reference_definitions()`. Each definition's span
    gives one `MdDest` link, used or not.
  - Bare URLs: for each paragraph or other inline container, regex-scan its *source* slice. Text
    events can't be used, because pulldown-cmark splits them at `_`, `(` and entities. The scan
    skips sub-ranges covered by code, link destinations and HTML events. A URL ends at whitespace,
    `"`, `'`, `<` or `>`, then trailing `.,;:!?` is trimmed, and a trailing `)` is trimmed unless
    its parens balance. This also covers shortcode arguments such as `src="…"`.
  - Html and InlineHtml events are regex-scanned for `href`/`src`.
  - Code events and code blocks are skipped.
- A destination that is not found verbatim is kept with `span` set to the event range and
  `verbatim: false`, so it is checked but never applied (R23).

**Check** (`src/check.rs`):
`async fn check_all(urls: Vec<Url>, cfg, deadline: Instant) -> HashMap<Url, Verdict>`.

- Uses `tokio` + `reqwest` (rustls). Redirects are off (`Policy::none()`) and followed by hand, at
  most 10 hops, with each hop recorded.
- Each body is read up to 256 KB.
- Each host has a `Mutex<Instant>` gate enforcing `host_interval_ms`; a global semaphore of 8
  limits concurrent hosts. Requests for one host run sequentially.
- `run` sets one `deadline = start + deadline_s` before main-flow step 4 and passes it to
  `check_all` and `snapshot`, which stop at it (`tokio::time::timeout_at`). URLs not finished by
  then get `Unverifiable("deadline")`; lookups not finished get `NoFix` "deadline" (R15).

```rust
struct Hop { url: Url, status: u16 }
enum Verdict {
  Ok { chain: Vec<Hop> },
  Permanent { chain: Vec<Hop>, fix: Url },
  Temporary { chain: Vec<Hop>, final_url: Url },
  Dead { reason: String },            // "404", "410", "dns"
  Suspect { chain: Vec<Hop>, reason: String },
  Unverifiable { reason: String },
}
```

- Verdicts derive `Serialize`, so a later state file is purely additive.
- Classification order:
  1. final status 404/410 or DNS failure (R7);
  2. transport errors and the listed unverifiable statuses (R14);
  3. redirect-to-root (R10);
  4. probe (R11);
  5. title phrases (R12);
  6. redirect kind (R8, R9), or no redirect at all: Ok (R14);
  7. anything left is unverifiable (R14 catch-all).
- The probe cache is keyed by (host, parent path) and stores the probe's final URL, status, title
  and shingle set.

```text
Inside check.rs: one distinct Url (deadline_s wraps the run; unfinished Urls → Unverifiable)

  Url
   │
   ▼
  host gate ....... ≤ 1 request in flight per host, starts ≥ host_interval_ms apart,
   │                ≤ 8 hosts at once; probes and re-checks wait here too
   ▼
  fetch chain ..... GET with User-Agent and timeout_ms, redirects off; follow Location by
   │                hand, record each Hop{url, status}, stop after 10 hops; body ≤ 256 KB;
   │                retry timeout, 5xx, connection error ×2; 429 ×1 after Retry-After ≤ 30 s
   ▼
  dead re-check ... final 404/410 or DNS failure: wait recheck_delay_ms, fetch again;
   │                the second answer is the one classified
   ▼
  probe ........... 2xx on a non-root path, soft404 on for the host: fetch
   │                <parent>/<24-char token> once per (host, parent path), cached
   ▼
  classify ........ first match in the classification order above ──► Verdict
```

**Archive** (`src/archive.rs`): `async fn snapshot(url, date, deadline) -> Result<Url, NoFix>`.

- Two CDX queries against `{archive_url}/cdx/search/cdx`, both with `url=<u>`,
  `filter=statuscode:200`, `fl=timestamp,original` and `output=json`:
  1. `to=<YYYYMMDD>&limit=-1`, the latest capture on or before the date;
  2. if that is empty, `from=<YYYYMMDD>&limit=1`.
- All archive requests go through one shared gate (`archive_interval_ms`) and get 3 attempts.
  `NoFix` is "no archived copy", "archive unreachable" or "deadline".

```text
Data model: held in memory for one run; nothing persists between runs

┌──────────────────┐ 1        * ┌────────────────────────────┐ *          1 ┌──────────────────┐
│ Page             ├───────────►│ Link   one per occurrence  ├─────────────►│ Url              │
│ path, date       │  has       │ url, line, span, ctx,      │ same url     │ distinct, with   │
└──────────────────┘            │ verbatim                   │ minus #frag  │ no #fragment     │
 owner: extract.rs              └─────────────┬──────────────┘              └───────┬──────────┘
                                              │ 1                                   │ 1
                                              │ if not ok                classified │
                                              ▼ 0..1                                ▼ 1
                                ┌────────────────────────────┐ verdict of   ┌──────────────────┐
                                │ Finding  per non-ok link   │◄─────────────┤ Verdict          │
                                │ link, verdict, and a fix   │ its Url      │ class, hops,     │
                                │ or a manual-fix reason     │              │ reason           │
                                └────────────────────────────┘              └──────────────────┘
                                owner: lib.rs::run                          owner: check.rs
fix: Permanent → last 301/308 target (from the Verdict);
     Dead, Suspect → archive.rs snapshot for (Url, page date), one lookup per pair
```

**Report** (`src/report.rs`): plain text to stdout; errors go to stderr. Entry format:

```
content/posts/x/index.md:12  DEAD 404  https://old.example/a
    fix: https://web.archive.org/web/20210101000000/https://old.example/a
checked 87: ok 80, permanent 3, temporary 1, dead 2, suspect 1, unverifiable 0, ignored 0
```

**Apply** (`src/apply.rs`): the only module that writes files.

- Edits are grouped per file. First, findings that are `verbatim: false` or whose fix cannot be
  written per `Ctx` become "manual fix" and leave the edit list (R23).
- Then each file is re-read, and every remaining span is verified to still hold the original URL
  (R22).
- Edits are applied from the last span backwards, then written to `<file>.rotguard.tmp` in the
  same directory and renamed over the original.

```text
State: one link occurrence through a run

[extracted] ──not absolute http(s), or host = site_url host──► [skipped] (not counted)
     │      ──matches an ignore regex──► [ignored] (counted)
     │ external
     ▼
[checked] ──Ok──► [ok] (summary only)
     │    ──Temporary or Unverifiable──► [reported, no fix]
     │    ──Suspect──► archive ──► [reported: snapshot or manual fix, never applied]
     │    ──Dead─────► archive ──no capture, unreachable, or deadline──► [manual fix]
     │                  │ snapshot found
     │                  ▼
     └────Permanent──► [has fix] ──no --apply──► [reported with fix]
                        │ --apply, every span in the file still holds its url,
                        │ and the fix can be written in that position
                        ▼
                  [rewritten] (counts as fixed)

  [has fix] ──--apply, url not verbatim or fix can't be quoted──► [manual fix]  (checked first)
  [has fix] ──--apply, any other span being applied in the file changed──► [changed during run]
```

**Main flow** (`lib.rs::run`):
1. load config;
2. glob and extract;
3. dedupe external URLs;
4. `check_all`;
5. look up the archive for dead and suspect occurrences;
6. build the findings;
7. if `--apply`, apply;
8. print the report;
9. exit code.

```text
Flow: a dead link from discovery to rewrite (main path; b, c = failure branches)

 1. run ──path, text──► extract ──Page{date, links: [Link{url, line, span}]}──► run
 2. run ──distinct Urls──► check ──GET through the host gate (HTTP(S))──► linked host
 3. linked host ──404──► check ──wait recheck_delay_ms, GET again──► linked host ──404──►
    check ──Verdict::Dead──► run
    3b. second answer is 2xx ──► classified like any 2xx (R8–R13), not dead (S10)
 4. run ──(Url, page date)──► archive ──CDX to=<date>, limit=-1 (HTTPS, JSON)──► Wayback
 5. Wayback ──latest capture on or before the date──► archive ──snapshot Url──► run
    5b. none before the date ──► CDX from=<date>, limit=1: the earliest capture after it
    5c. no capture, 3 failed attempts, or deadline ──► "manual fix: <reason>", never applied
 6. run ──Findings (--apply only)──► apply ──set aside unwritable fixes (R23), re-read file:
    every remaining span still equals its url
 7. apply ──replace spans last-first, write <file>.rotguard.tmp, rename over original──► run
    7b. any remaining span differs ──► whole file unchanged, reported "changed during run"
 8. run ──Findings, summary──► report ──► stdout; exit 0 if no finding is left unfixed
```

**External dependencies and how each fails:**

| Dependency | Fails how | Effect |
|---|---|---|
| Target sites | down, slow or blocking | unverifiable, never dead (R14) |
| Wayback CDX | down or rate-limited | "manual fix: archive unreachable" (R17); the run still finishes |
| DNS | NXDOMAIN | dead, after the re-check (R7) |
| Local DNS | outage | every host looks NXDOMAIN; both attempts fail and the run flags all dead. Accepted: the dry run shows it before any `--apply` |

## Decisions

### D1: Dry-run report by default, `--apply` edits sources in place
Options: report only; report + `--apply`; patch file. Chosen: report + `--apply` (user). Why:
matches the site's existing "dry run by default, `--apply` to write" scripts; git is the undo.
Cost if wrong: low. A `--patch` output can be added on top of the same edit list.

### D2: Dead links are fixed with the Wayback snapshot closest to the page date
Options: Wayback snapshot; flag only; ask interactively. Chosen: Wayback snapshot, preferring
captures on or before the page date (user). Why: that snapshot is most likely what the author
read when they linked it. Cost if wrong: a snapshot that already shows a different page. The
report prints the timestamp so the user can eyeball it before `--apply`.

### D3: Local CLI, CI-ready exit codes; no scheduled bot in v1
Chosen by user. Cost if wrong: low. A CI job is a few YAML lines calling the binary. The weekly
GitHub Action that opens fix PRs is parked in `docs/backlog.md` for v2.

### D4: Scan markdown sources, own reqwest checker, probe-based soft-404, stateless runs
Options: heuristics-only soft-404; wrap lychee-lib; add a run-state file. Chosen: the
architect's recommended package (user).
Why:
- `--apply` needs exact source spans, which built HTML cannot give.
- lychee-lib's `Status` exposes no per-hop codes or bodies, so it can't separate 301 from 302 or
  detect soft-404s.
- The random-path probe catches friendly soft-404s that keyword heuristics miss.
- A run takes about a minute, so a cache buys nothing yet.
Tradeoff: we write about 500–800 lines that lychee already has (TLS quirks, odd hosts), plus
about 37 extra probe requests per run.
Cost if wrong:
- A host with per-path "did you mean" pages fools the probe. Fix: `soft404 = false` for that host.
- A flaky host flaps in CI. Fix: the run-state file parked in `docs/backlog.md`; verdicts are
  already serialisable.
- Links that live in templates are missed. Fix: add an HTML discovery mode later.

### D5 (assumption): Redirect policy
Only a leading run of 301/308 hops is rewritten. A chain that starts with a 302/303/307 is
reported but doesn't fail the run unless `--strict` is given. Temporary redirects are often
locale or login hops the user cannot fix. Cost if wrong: a stale 302 lingers in the content
until someone looks at a `--strict` run.

### D6 (assumption): Soft-404 suspects and unverifiable links are never auto-applied
False positives cost trust. Suspects fail the run (exit 1) and show a proposed snapshot. The user
either edits the link by hand or adds an `ignore` or `soft404 = false` entry. Unverifiable links
(401/403/429/999, 5xx, timeouts) never count as dead. Cost if wrong: a few manual edits.

### D7 (assumption): Every http(s) string in front matter is a candidate; external = host ≠ `site_url` host
This is zero-config across sites and covers `source`, `discussedOn` and `featuredOn` on
rdiachenko.com. Self-links such as `canonicalUrl` drop out as internal. Cost if wrong: a front
matter URL that isn't a link (e.g. an API base in a code example field) gets checked. Fix: add
it to `ignore`.

### D8 (assumption): Images and raw-HTML `href`/`src` are checked too
They are outbound resources that rot in the same way. Cost if wrong: one extra request per image
URL.

### D9 (assumption): `robots.txt` is not consulted
This is common practice for link checkers verifying published links. Politeness comes from one
request per host per second and an honest User-Agent. Cost if wrong: a host blocks us, and its
links show as unverifiable.

### D10: Rust, one static binary
Options: Rust; Go; TypeScript (Node); Python. Chosen: Rust (user). Why: `pulldown-cmark`'s
offset iterator gives the exact byte spans `--apply` needs, `reqwest` with redirects off lets us
record every hop, and one binary drops into CI with no runtime. Crates: `tokio`, `reqwest`
(rustls), `pulldown-cmark`, `toml`, a YAML crate, `serde`, `regex`, `globset`/`walkdir`, `clap`,
`chrono`; dev-only `wiremock` and `tempfile`. Cost if wrong: low. No datastore or service
depends on the language.

## Risk paths

These always escalate to the user:
- `.github/**`, `.githooks/**`, `scripts/**`;
- `Cargo.toml`, `Cargo.lock`;
- `**/*secret*`, `.env*`;
- `.whittle/gates.toml`, the gate config itself;
- `src/apply.rs`, the only code that writes to user content.

## Fitness rules

- Only `src/apply.rs` writes files:
  `! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'`
- Tests never hit the real network. Every URL literal in `tests/` is local or uses a reserved TLD:
  `! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'`
- No private data in tracked files: `scripts/privacy-check.sh --all`

## Review changes

- Requirement R2: candidates limited to absolute http(s) URLs with a host; `mailto:`, relative, `#` and `//` are skipped silently (S40). R4 says what `ignored` counts.
- Extract: front-matter spans are now the exact string token between its own delimiters, not every occurrence, so comments and prefixes are never rewritten (S42).
- Extract: the body is parsed after the front matter, with offsets added, so spans are file-absolute.
- Extract: reference definitions are read from `reference_definitions()` (one link per definition), and reference-style uses are skipped. pulldown-cmark emits no event for definitions.
- Extract: bare URLs are scanned over source slices instead of Text events, which split at `_`, `(` and entities. A scenario now covers shortcode arguments (S39).
- Requirement R7: dead uses the final status after redirects (301 → 404 is dead). R14 gains a catch-all: other 4xx, a 3xx without Location, loops and over 10 hops are unverifiable (S41). The classification order was updated to match.
- Requirement R9: temporary needs a non-suspect 2xx end, so 302 → `/` is a suspect, as R10 says (S43).
- Requirement R11: similarity is explicitly computed on the first 256 KB, with the tradeoff stated.
- Requirement R19: the entry format is defined per class; the summary counts distinct URLs (S14 updated).
- Design: `Link` gains the `verbatim` field that the extraction rules relied on.
- Round 2 (diagrams and gates.toml): R14 now says a plain 2xx with no redirect and no suspect sign is ok; classification step 6 matches. Before, everything fell through to unverifiable.
- Round 2: the deadline covers checking and archive lookups. `run` passes one `deadline` to `check_all` and `snapshot`, and lookups it cuts off get "manual fix: deadline" (R15, S44).
- Round 2: unwritable or non-verbatim fixes are set aside as "manual fix" before the R22 span check, so they no longer mark the whole file "changed during run" (R22, Apply, S36, state and flow diagrams).
- Round 2: R23 also refuses fixes with unbalanced parentheses in a link destination (S45), or `<`/`>` in an autolink.
- Round 2: Scope says redirects are rewritten to the last leading 301/308 target, matching R8 and S13.
- Round 3: the first external review failed (codex exit 4), so the fixes below came from the builder check alone. The rerun reviewed the round-2 and round-3 fixes together; its one finding (no scenario for `<`/`>` in R23) was dropped, because fixes are `Url` values that percent-encode those characters.
- Round 3: R23's first clause now keys on the extraction-time `verbatim: false` flag, so a span changed during the run reaches R22's "changed during run" (S34) instead of becoming a manual fix.
- Round 3: R23's markdown rules are one bullet keyed on `Ctx::MdDest` (inline, definition or autolink), since `Ctx` has no autolink variant to tell two rules apart.

## Open for you

none
