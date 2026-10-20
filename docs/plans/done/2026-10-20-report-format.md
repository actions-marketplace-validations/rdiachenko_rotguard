# Plan: Structured report formats
Design: docs/design/report-format.md
Risk paths: Cargo.toml, Cargo.lock — T1 adds `serde_json` as a direct dependency (D2)
Diff estimate: 200

Order: the dependency first, so the acceptance tests can parse JSON; then the tests; then the
flag with only `text`, the JSON report and the HTML report; then the README.

Rules for every task:
- The text report must not change by a byte (R1). Exit codes, stderr and `--apply` stay as they
  are (R6).
- Only `src/apply.rs` writes files (fitness rule): reports go to stdout only. Tests never hit the
  real network.
- Existing acceptance tests are locked. New ones live in `tests/acceptance/format.rs`.
- No dependency beyond `serde_json`.

### T1: Add serde_json as a dependency
- Files: Cargo.toml, Cargo.lock
- Interfaces: Produces `serde_json` as a direct `[dependencies]` entry, at the version already in
  Cargo.lock (1.0.154; `cargo add serde_json@1.0.154` or the same line by hand). It is visible to
  src/ and to the tests. No code uses it yet, and Rust doesn't warn on an unused crate dependency.
- Test: none of its own; `cargo test` and clippy stay green.
- Covers: none
- Commit: Add serde_json as a dependency
- Review: skip

### T2: Acceptance tests
- Files: tests/acceptance/format.rs, tests/acceptance/main.rs
- Interfaces: Consumes the harness in tests/acceptance/support.rs (`World`, `Routes`, `redirect`,
  `page`, `status`, `Cdx`, `Capture`, `w.archive`, `w.url`, `w.snapshot_url`, `w.log`,
  `w.site.write`, `w.run`, `Out`) and `serde_json::Value` for parsing.
- Test: one test per scenario, named `format_s1_…` to `format_s8_…`. All eight fail today, because
  clap rejects the unknown `--format` argument, so every one is marked
  `#[ignore = "pending: S<n>"]`.
  - S2: the dead finding's fix URL is `w.snapshot_url("20200101000000", "/gone")`.
  - S3: the YAML front matter is `---` / `source: "M/old"` / `---`.
  - S4: rows are counted as the `<tr` occurrences between `<tbody>` and `</tbody>`.
  - S6: `M/x` is served for the path `/x`.
  - S7: the test asserts the exit code is 1.
  - S8: give the site a real link first, `post.md` with `[x](M/gone)` and
    `w.serve(Routes::new().fixed("/gone", status(404)))`. Then "no request" is
    `w.log.all().is_empty()`: without a served route nothing is logged, and the check would pass
    even if rotguard sent requests.
  - main.rs gains `mod format;`. It is locked: open it with `whittle-gates amend` first.
- Covers: S1, S2, S3, S4, S5, S6, S7, S8
- Commit: Add acceptance tests for report formats

### T3: Add the --format flag
- Files: src/lib.rs, src/report.rs
- Interfaces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)] pub enum Format { Text }` in
    src/lib.rs.
  - `Args` gains `#[arg(long, value_enum, default_value_t = Format::Text)] pub format: Format`.
  - `pub fn render_as(format: Format, findings: &[Finding], summary: &Summary) -> String` in
    src/report.rs: `Text` calls `render`.
  - `run` prints `report::render_as(args.format, …)` where it printed `report::render(…)`.
- Test: un-pends S1.
- Covers: R1, S1
- Commit: Add the --format flag

### T4: Print the report as JSON
- Files: src/lib.rs, src/report.rs
- Interfaces:
  - Adds `Format::Json`.
  - In src/report.rs, the private `Row` and `FixView` views from the design:
    - `FixView` is `#[derive(Serialize)] #[serde(tag = "kind", rename_all = "lowercase")]`.
    - `Fix::None` maps to `None`; `Proposed(u)` and `Applied(u)` map to the matching variant;
      `Manual(r)` maps to `Manual { reason: r.to_string() }`.
  - `rows(findings) -> Vec<Row>` takes the non-ok findings in the text order (sorted by file
    then line).
  - `Summary` gains `Serialize`.
  - `render_json` builds `{"findings": rows, "summary": summary}` with `serde_json` (pretty) and
    returns it without a trailing newline; `println!` adds the newline.
  - Rows by class:
    - `url` is `link.url` as written.
    - `chain` is the verdict's chain for permanent, temporary and suspect, and empty for dead
      and unverifiable.
    - `reason` is set for dead, suspect and unverifiable.
    - `final_url` is set for temporary only.
- Test: a report.rs unit test with one row per class and per fix kind, comparing
  `serde_json::Value`s. Un-pends S2, S3 and S7.
- Covers: R2, R3, R6, S2, S3, S7
- Commit: Print the report as JSON

### T5: Print the report as HTML
- Files: src/lib.rs, src/report.rs
- Interfaces:
  - Adds `Format::Html`.
  - `render_html(findings, summary) -> String` in src/report.rs, built from the same `rows`. It
    produces, in order: `<!doctype html>`, `<meta charset="utf-8">`,
    `<title>rotguard report</title>`, one `<style>`, a `<p>` with the text summary line, then
    either `<table><thead><tr><th>Location</th><th>Class</th><th>URL</th><th>Fix</th></tr></thead><tbody>…</tbody></table>`
    or `<p>No broken links.</p>`.
  - Each row's cells:
    - Class is the text header wording (`PERMANENT REDIRECT 301 200`, `DEAD 404`, …).
    - URL is `<a href="u">u</a>` of the URL as written.
    - Fix is the text detail line, with its URL as a link (`fix: <a …>`, `final: <a …>`), or
      `manual fix: <reason>`. It is empty for unverifiable rows.
  - `fn escape(s: &str) -> String` maps `&`, `<`, `>`, `"` and `'` to `&amp;`, `&lt;`, `&gt;`,
    `&quot;` and `&#39;`, and is applied to every interpolated value, `href` included.
- Test: report.rs unit tests: `escape` on all five characters; a page with a finding of each
  class (the cells and the `<tbody>` row count); the no-findings page. Un-pends S4, S5, S6 and
  S8.
- Covers: R4, R5, R7, S4, S5, S6, S8
- Commit: Print the report as HTML

### T6: Document --format in the README
- Files: README.md
- Interfaces: none (docs).
  - In the Use section, add `rotguard --format json` and `rotguard --format html > report.html`
    lines with one-line comments.
  - Add one short paragraph naming the JSON keys (`findings`, each with `file`, `line`, `url`,
    `class`, `chain`, `reason`, `final_url` and `fix` with its `kind`; and `summary`).
  - In the sample report, change `PERMANENT REDIRECT 301` to `PERMANENT REDIRECT 301 200`.
  - Leave the `TEMPORARY REDIRECT 302` sample alone; it is a parked backlog line.
- Test: none (docs).
- Covers: none
- Commit: Document --format in the README
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
