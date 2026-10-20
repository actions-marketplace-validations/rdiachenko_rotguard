# Plan: Progress output and a config reference
Design: docs/design/progress-and-config.md
Risk paths: none — no task touches src/apply.rs, Cargo.toml, Cargo.lock or scripts/
Diff estimate: 350 (about 120 lines of code plus docs/config.md and the README edit)

Order: a no-op `progress` interface first, so the acceptance tests compile; then the tests; then
the `Progress` output, the check-phase wiring and the archive-phase wiring; then the two docs.

Rules for every task:
- No new dependency. Terminal detection is `std::io::IsTerminal`. Only `src/apply.rs` writes
  files (fitness rule). Tests never hit the real network.
- Progress goes to stderr only; stdout's report must not change (R1). Every write to stderr
  ignores its error (`let _ = …`): progress never fails a run.
- Wording and plural forms live only in `src/progress.rs`, exactly as R2–R4 give them.
- Existing acceptance tests are locked. New ones live in `tests/acceptance/progress.rs`.

### T1: Add the progress module interface
- Files: src/progress.rs, src/lib.rs
- Interfaces: Produces `pub mod progress;` in src/lib.rs, with
  `pub enum Phase { Check { hosts: usize }, Archive }`,
  `pub struct Progress<W: Write>`, `Progress::stderr() -> Progress<io::Stderr>` (live =
  `io::stderr().is_terminal()`), `Progress::new(out: W, live: bool)`,
  `start(&mut self, phase: Phase, total: usize)`, `tick(&mut self, done: usize)`,
  `finish(&mut self)`. In this task the three methods write nothing. Each item gets a doc
  comment saying what it will print (the R2–R4 text). To keep `clippy -D warnings` green with
  no behaviour, the unused parameters are named `_phase`, `_total` and `_done`, and the struct's
  fields carry `#[expect(dead_code, reason = "printed from T3")]`. T3 removes both.
- Test: none of its own (no behaviour yet); `cargo test` and clippy stay green.
- Covers: none
- Commit: Add the progress module interface
- Review: skip

### T2: Acceptance tests
- Files: tests/acceptance/progress.rs, tests/acceptance/main.rs
- Interfaces: Consumes T1's `rotguard::progress` API (S4 only) and the harness in
  tests/acceptance/support.rs (`World`, `Routes`, `status`, `page`, `Cdx`, `Capture`,
  `w.archive`, `w.site.write`, `Out.stderr`, `Out.code`).
- Test: one test per scenario, named `progress_s1_…` to `progress_s7_…`.
  - S1, S2, S4, S6 and S7 fail today, so they're marked `#[ignore = "pending: S<n>"]`.
  - S3 and S5 already hold (nothing prints today), so they're regression guards with no marker.
  - S4 drives the library (`Progress::new(&mut buf, true)` over a `Vec<u8>`), and its time part
    is matched with a pattern (`[0-9]+s`).
  - S6 and S7 find docs/config.md and README.md via `env!("CARGO_MANIFEST_DIR")`. They take the
    last TOML block of docs/config.md, or the README Configure section's TOML block, and write it
    over the harness's `rotguard.toml` with `w.site.write`. They put one link-free page where
    that block's `content` globs read it (`content/a.md`). `Site::write` doesn't create parent
    directories, so the test first calls `std::fs::create_dir_all(w.site.path().join("content"))`.
    They assert exit 0 plus the key, heading and link checks of their scenario.
  - main.rs gains `mod progress;`, and its header comment notes that `progress.rs` also drives
    the library. main.rs is locked: open it with `whittle-gates amend` first.
- Covers: S1, S2, S3, S4, S5, S6, S7
- Commit: Add acceptance tests for progress and config docs

### T3: Print progress in plain and live mode
- Files: src/progress.rs
- Interfaces: Implements T1's methods, and removes T1's `_`-prefixed parameter names and its
  `#[expect(dead_code)]` attributes.
  - Plain mode (`live == false`): `start` prints the R2/R3 start line plus `\n`, `tick` prints
    nothing, `finish` prints the end line plus `\n`.
  - Live mode: `start` prints `\r<label>: 0/<N> done`, `tick(d)` prints `\r<label>: <d>/<N> done`,
    and `finish` prints `\r\x1b[2K` followed by the end line and `\n`. The label is
    `checking links` or `looking up snapshots`.
  - The start time is a `std::time::Instant`, taken in `start`; T is `elapsed().as_secs()`.
  - Nouns: link/links, host/hosts, snapshot/snapshots, singular only for a count of 1.
- Test: unit tests in progress.rs, one table of `(phase, total, ticks, live, expected text)`
  covering plain and live mode for both phases, singular and plural, and N = 0. The time digits
  are matched with a pattern. Un-pends S4.
- Covers: R4, S4
- Commit: Print progress in plain and live mode

### T4: Show progress while checking links
- Files: src/check.rs, src/lib.rs
- Interfaces:
  - `check::check_all(urls, cfg, deadline, mut on_done: impl FnMut(usize))` calls `on_done(n)`
    after each `join_next` result, with n the count done so far. A deadline-cut URL counts.
  - The existing callers in check.rs's unit tests pass `|_| {}`.
  - In lib.rs, `run` creates `Progress::stderr()` after config load and `read_pages` succeed, and
    passes `&mut progress` to `check_pages`.
  - `check_pages` counts the distinct hosts of `distinct` (by `host_str()`), calls
    `progress.start(Phase::Check { hosts }, distinct.len())`, then `check_all` with
    `|d| progress.tick(d)`, then `progress.finish()`.
- Test: a check.rs unit test that `on_done` is called once per URL with 1..=N; un-pends S1. S3
  and S5 stay green.
- Covers: R1, R2, R5, S1, S3, S5
- Commit: Show progress while checking links

### T5: Show progress during archive lookups
- Files: src/lib.rs
- Interfaces:
  - `look_up_snapshots(lookups, cfg, deadline, mut on_done: impl FnMut(usize))` calls
    `on_done(n)` after each `join_next` result. A panicked lookup counts as done too.
  - `check_pages` reads `lookups.len()` before the call. When it is not 0, it wraps the call in
    `progress.start(Phase::Archive, n)` and `progress.finish()`, with `|d| progress.tick(d)`.
  - With no lookups, nothing is printed for the archive phase.
- Test: un-pends S2; S3 stays green.
- Covers: R3, S2, S3
- Commit: Show progress during archive lookups

### T6: Write the config reference
- Files: docs/config.md
- Interfaces: none (docs).
  - One `###` heading per key, in src/config.rs order: `site_url`, `content`, `date_key`,
    `user_agent`, `ignore`, `not_found_phrases`, `archive_url`, `host_interval_ms`,
    `archive_interval_ms`, `recheck_delay_ms`, `timeout_ms`, `deadline_s`, then
    `[hosts."<name>"]` with `soft404`.
  - Each entry gives the type, the default, the valid values, the exact effect and the error for
    an invalid value. Every statement is checked against the code before it is written. Facts
    already verified:
    - `site_url`: compared by host only, so scheme and port are ignored. There is no `www.`
      folding and no subdomain match.
    - `content`: globs are relative to the site directory; `*` stays within one segment and `**`
      crosses segments.
    - `ignore`: an unanchored regex search on the URL as written, fragment included. It runs
      after the internal-host check. URLs that don't parse are skipped too. Ignored URLs count
      as `ignored` in the summary.
    - `not_found_phrases`: a case-insensitive substring match on the page `<title>`, with
      whitespace in a phrase collapsed and empty phrases ignored. It applies only to 2xx pages
      on hosts with soft-404 on, and is skipped when the probe answered 404 or 410. A match
      makes the link SUSPECT.
    - `user_agent`: sent on checks, probes and archive queries.
    - `host_interval_ms`: start-to-start spacing per host, counting redirect hops, retries,
      rechecks and probes.
    - `archive_interval_ms`: one global spacing between archive requests.
    - `recheck_delay_ms`: the wait before re-fetching a dead URL.
    - `timeout_ms`: per request, not per redirect chain; the same value applies to archive
      requests.
    - `deadline_s`: covers checking and archive lookups together, at most 86400.
    - `[hosts]`: keys are lowercased and matched exactly against the URL's host.
    - Errors: an unknown key, a wrong type or a missing `site_url` exits 2 with
      `invalid rotguard.toml`.
  - The page ends with one TOML block listing every top-level key at its default, with
    `site_url = "https://example.com"`.
- Test: un-pends S6.
- Covers: R6, S6
- Commit: Write the config reference

### T7: Link the config reference from the README
- Files: README.md
- Interfaces: none (docs). In the Configure section, the example uses
  `[hosts."www.goodreads.com"]`. The "Other keys" sentence is replaced by a link to
  `docs/config.md` for every key's type, default and effect.
- Test: un-pends S7.
- Covers: R7, S7
- Commit: Link the config reference from the README
- Review: skip

## Verification contract
```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'
! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'
scripts/privacy-check.sh --all
```
