# Plan: A fix `--apply` couldn't write is a hand fix
Design: docs/design/apply-failed-fix.md
Risk paths: src/apply.rs — T2, `apply` marks a failed file's proposed fixes as `WriteFailed`
Diff estimate: 22

### T1: Acceptance tests
- Files: tests/acceptance/apply_failed.rs (new), tests/acceptance/main.rs
- Interfaces: Consumes the CLI through `tests/acceptance/support.rs` (`World`, `Routes`,
  `redirect`, `page`, `w.site.write`, `w.site.read`, `w.run`, `Out::entries`, `Out::assert_code`).
- Test: one test per scenario S1–S3, named `apply_failed_s<n>_…`, each marked
  `#[ignore = "pending: S<n>"]`. The site: `a.md` holding `[a](M/old)\n`, `post.md` holding
  `[b](M/old)\n`, and `post.md.rotguard.tmp` holding `x` (written with `w.site.write`), with
  `M/old` → 301 → `M/new` → 200.
  - S1 asserts the exact stdout, exit 2, `a.md` rewritten, `post.md` and the `.tmp` file
    unchanged, and stderr containing `rotguard: applying fixes: post.md` and
    `remove it and run again`.
  - S2 picks the entry at loc `post.md:1` from `out.entries()` (M/old is in two files, so
    `Out::entry(url)` panics) and checks its detail is `manual fix: write failed` and that no
    `post.md` entry has a `fix:` detail.
  - S3 parses stdout as JSON and finds the rows by `file` and `line`.
  Today S1 fails on the `Fixes to apply` section, S2 on `fix:`, S3 on `"kind": "proposed"`.
  `main.rs` is locked: open it with `whittle-gates amend` and add `mod apply_failed;` only.
- Covers: S1, S2, S3
- Commit: Add acceptance tests for a failed apply

### T2: Report a failed write as a hand fix
- Files: src/lib.rs, src/apply.rs, src/report.rs
- Interfaces: Produces `ManualReason::WriteFailed` (Display `write failed`; `md_reason` gives
  `rotguard couldn't write the file; its error message says why`). `apply::apply` keeps its
  signature; after `apply_file` returns `Err` for a file, each of that file's findings still
  holding `Fix::Proposed` becomes `Fix::Manual(ManualReason::WriteFailed)`.
- Test: add the `WriteFailed` row to `manual_reasons_print_their_report_strings` (src/lib.rs).
  In `one_unreadable_file_does_not_stop_the_others` (src/apply.rs:575-578) the failed file's
  assertion becomes `Fix::Manual(ManualReason::WriteFailed)`. Update `apply`'s doc comment
  ("its findings keep `Fix::Proposed`" → "its proposed fixes become `Manual(WriteFailed)`").
  Add one unit test in src/apply.rs: a file with two links, one whose fix is writable and one
  whose fix is `https://a.test/a)b` in a markdown destination (`CannotQuote`), plus a leftover
  regular `<file>.rotguard.tmp`. After `apply`, the first is `Manual(WriteFailed)` and the second
  stays `Manual(Unwritable(CannotQuote))`, so the marking step touches only `Proposed` fixes.
  Un-pends S1, S2, S3.
- Covers: R1, R2, R3, S1, S2, S3
- Commit: Report a failed write as a hand fix

### T3: Docs
- Files: README.md, docs/architecture.md
- Interfaces: none
- Test: none. Read the branch's diff against `Base:` in .whittle/run/progress.md.
  - README.md `manual fix:` reasons list (~line 83-90): add `write failed`: rotguard couldn't
    write the file; stderr says why (for example a leftover `<file>.rotguard.tmp`), and it exits
    2.
  - docs/architecture.md Apply paragraph (~line 89-92): an I/O error leaves that file as it was,
    reports its fixes as `manual fix: write failed`, and exits 2. Add a Decisions line citing
    `docs/design/apply-failed-fix.md` D1.
- Covers: none
- Review: skip
- Commit: Update docs for a failed apply

## Verification contract
```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'
! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'
scripts/privacy-check.sh --all
```
