# Plan: Keep a redirect's noise out of the site
Design: docs/design/redirect-noise.md
Risk paths: none — every task changes `src/check.rs`, tests or docs.
Diff estimate: 130

### T1: Acceptance tests
- Files: tests/acceptance/redirect_noise.rs, tests/acceptance/main.rs, tests/acceptance/support.rs
- Interfaces: `support.rs` gains `Routes::with_re(pattern: &str, handler)`, a regex route whose
  handler gets the request and the hit count like `Routes::with` (exact path, handler); it
  reuses `Pattern::Regex` from `fixed_re`. `main.rs` registers `mod redirect_noise;`.
  `redirect_noise.rs` holds S1 to S33 with a file doc comment that lists them. All hosts are the
  mock M: `w.url(path)`. A redirect that differs from its source only by query is 1
  `Routes::with(path, |req, _| ...)` handler that branches on `req.url.query()`, because routes
  match by path. For a link with no query, `None` answers the 301 and `Some` answers the page.
  For a link that has a query (S2, S3, S4, S5, S20) the handler compares the exact query
  string: the link's own (`Some("id=5")` in S4) answers the 301 and anything else answers the
  page. A local `assert_kept` helper
  checks the design's "Kept": `out.entries_for(url)` is empty, `w.site.read("post.md")` equals
  what was written, `out.assert_summary` with `checked: 1, ok: 1` and every other count 0, and
  `out.assert_code(0)`. Rewrites assert `entry.verdict`, `entry.detail` (`fix: <url>`) and
  `w.site.read("post.md")` after `w.run(&["--apply"])`. Request counts: S1, S7, S16, S31 and S32
  use `w.log.hits(path).len()`; S26 filters `w.log.hits("/new")` by `query.is_none()`; S19
  filters `w.log.all()` by `path.starts_with("/us/")` and asserts exactly 2 requests, 1 with
  `query == None` and 1 with the 2 `utm_*` pairs, so a soft-404 probe at `/us/<token>` fails
  it. S19 and the scenarios built on its site give both `/us/getting-started` answers the same
  `page("Now Awin", ...)`. S25 gives the 2 answers different titles. S26 sets `timeout_ms` low
  (e.g. 300) and delays the no-query `/new` answer past it with `set_delay`, keeping the default
  deadline. S27 sets `deadline_s = Some(1)` and a `timeout_ms` above it (e.g. 5000) with a
  5 s delay, as `s24_deadline_marks_unfinished_urls` in `tests/acceptance/check.rs` does, and
  asserts `UNVERIFIABLE deadline`. S28 lists M's host in `no_soft404_hosts`. S29 uses
  `with_re("^/s/article/", ...)`: no query answers 301 to the same path plus
  `?language=en_US`, a query answers `page("Page not found", ...)` with the same body for every
  path, and the test asserts `entry.verdict.starts_with("SUSPECT")`. S30 runs
  `&["--strict", "--format", "json"]` and parses stdout with `serde_json`. S33 links
  `M/old#start`.
- Test: one test per scenario, named `redirect_noise_s<N>_…` after its S-ID. The scenarios that
  change behaviour each carry their own `#[ignore = "pending: S<N>"]`: S1, S2, S3, S6, S11, S12,
  S13, S15, S16, S17, S18, S19, S20, S21, S22, S23, S24, S25, S26, S27, S28, S30, S31, S33. The
  others pin behaviour the design keeps and hold before any code changes: S4, S5, S7, S8, S9,
  S10, S14 and S32 (rewrites 0.2.1 already makes) and S29 (0.2.1 already calls it `SUSPECT`).
  They carry no pending marker, pass at this commit and are regression tests from then on, as
  the 404-walls build kept its S6 to S9.
- Covers: S1, S2, S3, S4, S5, S6, S7, S8, S9, S10, S11, S12, S13, S14, S15, S16, S17, S18, S19, S20, S21, S22, S23, S24, S25, S26, S27, S28, S29, S30, S31, S32, S33
- Commit: Add acceptance tests for redirect noise

### T2: Keep links whose redirect only adds noise
- Files: src/check.rs
- Interfaces: 2 new pure functions, `fn locale_shaped(segment: &str) -> bool` (R2) and
  `fn is_noise(from: &Url, to: &Url) -> bool` (R1). `is_noise` compares `origin()`, then the
  decoded `query_pairs()` as multisets (every pair of `from` occurs in `to` at least as often),
  then the paths: identical path strings are noise; otherwise compare `path_segments()` with 1
  trailing empty segment dropped (`/` is the empty list): equal lists are noise only when `to`
  has more pairs than `from`; `to` being `from` with exactly 1 segment inserted at any position
  is noise when that segment is `locale_shaped`. Fragments are ignored. In `classify`'s
  `200..=299` arm, with `leading = permanent_run(chain)`: `clean` = the count of steps
  `chain[i] → chain[i + 1]`, from `i = 0` while `i < leading`, that are not noise, stopping at
  the first noise step. The root rule keeps its 0.2.1 tests (`last` is root, or `leading > 0` and
  `chain[leading]` is root) and adds `clean > 0 && is_root(&chain[clean])`, all under
  `host.soft404 && !is_root(&chain[0])`. Then `leading > 0 && clean == 0` gives
  `Verdict::Ok { chain }`, `leading > 0` gives `Verdict::Permanent { chain, fix:
  chain[clean].url }`, and the remaining arms are unchanged. `no_location_reason` and
  `permanent_run` are unchanged. Update the doc comments on `Verdict::Permanent.fix` and
  `classify` to say where the fix points.
- Test: unit tables in `src/check.rs` for `is_noise` and `locale_shaped`, holding every row of
  the 2 tables in the design's Details; rows in the `classify` tables for a noise first step
  (`Ok`), a real step then a noise step (fix is the middle URL) and `/promo` → `/` → `/en-gb/`
  (`Suspect` with soft404 on). The existing case "a root url redirected to the root is
  unaffected" in `redirects_to_the_root_classify_by_soft404_and_final_status` now expects `Ok`,
  as the design's Details says (changing this unit test is allowed: it is in `src/check.rs`,
  not a locked acceptance file); "a middle root hop keeps the permanent fix" stays as it is.
  Un-pends S1, S2, S3, S6, S11, S12, S13, S15, S16, S17, S18, S30, S31
- Covers: R1, R2, R3, R4, R5, R10, R11, R12, S1, S2, S3, S6, S11, S12, S13, S15, S16, S17, S18, S30, S31
- Commit: Keep links whose redirect only adds noise

### T3: Drop utm parameters a redirect added
- Files: src/check.rs
- Interfaces: `const TRACKING_PREFIXES: &[&str] = &["utm_"];` and
  `fn without_tracking(fix: &Url, link: &Url) -> Option<Url>` (R6, R9): split the fix's raw
  query on `&`, drop a piece when the percent-decoded text before its first `=` starts with a
  `TRACKING_PREFIXES` entry (ASCII case ignored) and equals no decoded key of the link's query,
  join the rest with `&` unchanged, `set_query(None)` when none is left, `None` when no piece
  was dropped. In `check_one`, a last step on every path that returns a `Verdict::Permanent`
  built from `fetched`: the early return for a host with `soft404 = false` (R13) and the
  `Soft404::NoEvidence` arm. When `without_tracking(&fix, url)` is `Some(stripped)`, call
  `fetcher.fetch(&stripped)` once (no retry, no soft-404 probe, no recheck). The fix becomes
  `stripped` when the result is `Ok(f)`, `classify(&Ok(f), &HostConfig::default())` is
  `Verdict::Ok`, and `title_of` of `f.body` equals `title_of` of `fetched.body` (both through
  `String::from_utf8_lossy`; 2 empty titles are equal). In every other case the verdict becomes
  `Verdict::Ok { chain }` with the link's own chain (R8). The step stays inside the existing
  `timeout_at(deadline, check_one)`, which gives R14 with no new code. `src/lib.rs` re-adds the
  link's fragment as today. Update the doc comment on `check_one`.
- Test: a unit table in `src/check.rs` for `without_tracking`: 2 added `utm_*` pairs leave no
  `?`; `id=7&utm_source=old&ref=x` gives `id=7&ref=x`; a `utm_source` the link's query has is
  kept while an added `utm_medium` goes; `UTM_Source` is dropped; a percent-encoded pair that
  stays is byte-for-byte unchanged; no `utm_*` pair gives `None`. Un-pends S19, S20, S21, S22,
  S23, S24, S25, S26, S27, S28, S33
- Covers: R6, R7, R8, R9, R13, R14, S19, S20, S21, S22, S23, S24, S25, S26, S27, S28, S33
- Commit: Drop utm parameters a redirect added

### T4: Docs
- Files: README.md, docs/architecture.md, docs/config.md, CHANGELOG.md, docs/backlog.md
- Interfaces: read the branch diff against `Base:` in `.whittle/run/progress.md`. README class
  table: the `PERMANENT REDIRECT` row as the design's Details "Docs" says. `docs/architecture.md`:
  Check flow step 5 (a permanent redirect proposes the URL before its first noise step, without
  `utm_*` the redirect added), a "Permanent fixes" paragraph after the soft-404 section (noise
  steps, where the fix points, the stricter root case, the strip and its proof), the first
  Decisions line ("can't be wrong") citing `docs/design/redirect-noise.md` D1 and D3, and a Hard
  parts entry for the noise rule naming `tests/acceptance/redirect_noise.rs`.
  `docs/config.md`: `host_interval_ms` lists the stripped-URL request among the requests it
  counts. `CHANGELOG.md` under `## [Unreleased]`: the 4 bullets the design's Details "Docs"
  lists. `docs/backlog.md`: delete the 2 `[idea]` lines the design covers ("Drop utm_* query
  parameters a redirect added…" from shape 404-walls and "Don't rewrite a permanent redirect
  that only localises the link…" from 2026-11-11); keep the 3 lines from shape redirect-noise.
  Don't touch `Cargo.toml`, the README install snippet or the version: the release happens in
  another session.
- Test: none
- Covers: none
- Review: skip
- Commit: Update docs for redirect noise

## Verification contract
```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'
! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'
scripts/privacy-check.sh --all
```
