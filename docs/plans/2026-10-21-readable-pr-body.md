# Plan: Readable fix PR body
Design: docs/design/readable-pr-body.md
Risk paths: action.yml — T3 changes the run step to `--apply --format markdown` and the job summary
Diff estimate: 185

### T1: Acceptance tests
- Files: tests/acceptance/markdown.rs (new), tests/acceptance/main.rs
- Interfaces: none
- Test: one test per scenario in the new `markdown.rs`, named `markdown_s<n>_…` after its S-ID
  and marked `#[ignore = "pending: S<n>"]`. S1–S7 drive the binary through `World::run`
  (tests/acceptance/support.rs); S8 and S9 run the action's `id: run` step with a helper of
  their own in `markdown.rs`, copied from the one in tests/acceptance/action.rs. That helper
  sets `GITHUB_STEP_SUMMARY` to an empty temp file for S8 and removes it with `env_remove`
  for S9. `main.rs` is locked by earlier designs: open it with `whittle-gates amend` and add
  `mod markdown;`, nothing else.
- Covers: S1, S2, S3, S4, S5, S6, S7, S8, S9
- Commit: Add acceptance tests for the markdown report

### T2: Markdown report format
- Files: src/lib.rs, src/report.rs
- Interfaces: Consumes `Finding`, `Fix`, `ManualReason`, `Verdict`, `Summary` and the helpers in
  src/report.rs (`reported`, `display_path`, `statuses`). Produces `Format::Markdown` (clap
  value `markdown`) and `pub fn render_markdown(findings: &[Finding], summary: &Summary) ->
  String` (no trailing newline), dispatched from `render_as`.
- Test: unit tests in src/report.rs:
  - every section with its heading counts and plurals;
  - row merging across lines;
  - each problem wording and each hand-fix reason;
  - the counts line, with zero counts left out;
  - R6 escaping, table-driven: each character of `` \ ` * _ [ ] < > ( ) ! | ~ & # `` in a
    page path comes out backslash-escaped; `\n` and `\t` become a space; a plain `https://`
    URL is written as `<url>`; a URL with each autolink disqualifier (space, `<`, `>`, `|`,
    backtick, backslash, a control character) and a non-http URL are written as escaped text,
    never as `<…>`.
  Un-pends S1–S7.
- Covers: R1, R2, R3, R4, R5, R6, R7, R8, S1, S2, S3, S4, S5, S6, S7
- Commit: Add a markdown report format

### T3: Use the markdown report in the action
- Files: action.yml, tests/acceptance/action.rs
- Interfaces: Consumes `rotguard --apply --format markdown` from T2. Produces the run step of the
  design's Details: the report file is rotguard's stdout, printed to the log and appended to
  `$GITHUB_STEP_SUMMARY` when it is set and not empty; outputs `exit-code` and `report` as
  before.
- Test: un-pends S8 and S9. The locked `tests/acceptance/action.rs` is amended here through
  `whittle-gates amend tests/acceptance/action.rs --reason "readable-pr-body D6"`, exactly as the
  design's "Amended locked tests (D6)" says, and nothing more:
  - `action_s1_fix_is_written_and_reported` expects S8's report text exactly;
  - `action_s2_hand_fix_exits_one_and_writes_nothing` expects
    `no archived copy in the Wayback Machine`;
  - `HEADING` and S1's `stdout` variable go.
  The amendment and the `action.yml` change land in one commit: each alone breaks a test the
  other fixes, so neither can pass the checks as its own commit. weekly-fix-pr's `action_s3`,
  `action_s4` and `action_s5` still pass.
- Covers: R9, S8, S9
- Commit: Use the markdown report for the PR body

### T4: Docs
- Files: README.md, docs/github-actions.md, docs/architecture.md, docs/design/weekly-fix-pr.md
- Interfaces: none
- Test: none. Read the branch's diff against `Base:` in .whittle/run/progress.md.
  - README: add `rotguard --format markdown` next to the `--format` lines (README.md:25-26).
  - docs/github-actions.md: replace the PR body description and sample with the markdown
    report, and say the run's summary page shows it. Keep the first yaml block unchanged, since
    weekly-fix-pr's S5 test reads it.
  - docs/architecture.md: `--format` lists markdown (lines 14, 55 and 108); the action paragraph
    says the PR body is the markdown report; add a Decisions line citing readable-pr-body D1
    and D4.
  - docs/design/weekly-fix-pr.md: add `superseded-by: docs/design/readable-pr-body.md` to its
    frontmatter (its R3 report layout is replaced).
- Covers: none
- Review: skip
- Commit: Update docs for the markdown report

## Verification contract
```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'
! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'
scripts/privacy-check.sh --all
```
