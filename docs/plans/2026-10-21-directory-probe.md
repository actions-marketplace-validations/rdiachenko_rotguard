# Plan: Probe a sibling for a link that ends in `/`
Design: docs/design/directory-probe.md
Risk paths: none
Diff estimate: 14

### T1: Acceptance tests
- Files: tests/acceptance/directory_probe.rs (new), tests/acceptance/main.rs
- Interfaces: Consumes the CLI through `tests/acceptance/support.rs` (`World`, `Routes::fixed`,
  `Routes::fixed_re`, `page`, `status`, `w.site.write`, `w.run`, `w.log.all()`, `Out::entry`,
  `Out::entries`, `Out::assert_code`).
- Test: one test per scenario S1–S4, named `directory_probe_s<n>_…`, each marked
  `#[ignore = "pending: S<n>"]`. Count probes from `w.log.all()` with each scenario's regex,
  not with `w.log.probes()`: `is_probe` reads the last path segment, which is empty for
  `/<token>/`. S4 runs with `--strict` and checks the `M/docs/` entry's verdict is `BLOCKED 403`.
  Today every test fails: S1 is SUSPECT (probe `/docs/<token>` matches the route), S2 is ok
  (probe `/blog/gone/<token>` answers 404), S3 sends three probes, S4 is UNVERIFIABLE (probe
  `/docs/<token>` answers 404). `main.rs` is locked: open it with `whittle-gates amend` and add
  `mod directory_probe;` only.
- Covers: S1, S2, S3, S4
- Commit: Add acceptance tests for directory probes

### T2: Probe a sibling for a path ending in /
- Files: src/check.rs
- Interfaces: `Probes::probe(&self, fetcher, url) -> Arc<ProbeResult>` keeps its signature.
  For a path that ends in `/` and isn't `/`, strip that one slash before `rsplit_once('/')`
  picks the parent, and probe `{parent}/{token}/`; other paths probe `{parent}/{token}` as
  today. `ProbeCache`'s key becomes `(String, String, bool)`: origin, parent, ends in `/`.
  Update the doc comments on `ProbeCache`, `Probes` and `probe`.
- Test: add a unit test in src/check.rs that a URL ending in `/` gets one probe matching
  `^/docs/[a-z0-9]{24}/$` for `/docs/real/` (modelled on
  `two_urls_under_one_path_share_one_probe_that_404s`). Un-pends S1, S2, S3, S4.
- Covers: R1, R2, R3, S1, S2, S3, S4
- Commit: Probe a sibling for a path ending in /

### T3: Docs
- Files: docs/architecture.md
- Interfaces: none
- Test: none. Read the branch's diff against `Base:` in .whittle/run/progress.md.
  - docs/architecture.md:80-83 (soft-404 paragraph): a link ending in `/` is probed as a
    sibling folder, `/a/<token>/`; probes are shared per origin, parent path and whether the
    path ends in `/`.
  - README.md and docs/config.md don't describe the probe's path; leave them.
- Covers: none
- Review: skip
- Commit: Update docs for directory probes

## Verification contract
```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'
! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'
scripts/privacy-check.sh --all
```
