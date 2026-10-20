# Plan: Weekly fix PR
Design: docs/design/weekly-fix-pr.md
Risk paths: action.yml — T2 creates it; the design names it under Touches (D8)
Diff estimate: 120

Order: the acceptance tests first; then `action.yml`, which un-pends S1 to S4; then
`docs/github-actions.md`, the site workflow, which un-pends S5; then the docs.

Rules for every task:
- rotguard's Rust code doesn't change: nothing under `src/`, no new dependency, flag or config
  key. `serde_norway` is already a dependency and the tests may use it.
- Only `src/apply.rs` writes files (fitness rule). Tests never hit the real network, and no test
  file holds an `http://` or `https://` literal other than 127.0.0.1, localhost or `.test`,
  `.example`, `.invalid` hosts.
- Existing acceptance tests are locked. New ones live in `tests/acceptance/action.rs`. A locked
  file changes only in the task that names it under `Amends:`; the orchestrator opens it with
  `whittle-gates amend` before the builder starts.
- The action's run step works under macOS's `/bin/bash` 3.2 and on ubuntu-latest: no
  `${var,,}`, no `mapfile`, no `mktemp -t` or `--tmpdir`. It holds no `${{ }}` expression.
- Use the exact strings from the design's Details: output names, the report path
  `$RUNNER_TEMP/rotguard-report.md`, the heading line, the branch name, the messages, the title.

### T1: Acceptance tests
- Files: tests/acceptance/action.rs (new), tests/acceptance/main.rs
- Amends: tests/acceptance/main.rs (adds `mod action;`)
- Interfaces: none. Use tests/acceptance/support.rs as it is: `World::new`, `w.serve(Routes)`,
  `Routes::fixed`, `redirect`, `status`, `w.url`, `w.site.path()`, `write`, `read`,
  `snapshot()`, `w.archive(Cdx::new(vec![]))` (A holds no captures by default). `World::new`
  writes `rotguard.toml` with `archive_url` pointing at A.
- Test: one test per scenario, `action_s1_…` to `action_s5_…`, each
  `#[ignore = "pending: S<n>"]`.
  - Find `action.yml` and `docs/github-actions.md` through `env!("CARGO_MANIFEST_DIR")`. Parse
    YAML with `serde_norway::Value`.
  - A helper runs the step the way GitHub does, for S1 to S3. It finds the step with
    `id: run` in `runs.steps` and writes its `run` text to a temp file. It runs that file with
    `/bin/bash` if it exists, else `bash`, as `bash --noprofile --norc -eo pipefail <file>`,
    with the site folder as cwd. It uses `tokio::task::spawn_blocking`, as `support::run` does.
    The environment:
    - `GITHUB_ACTION_PATH`: a temp folder whose `target/release/rotguard` is a symlink to
      `env!("CARGO_BIN_EXE_rotguard")`;
    - `GITHUB_OUTPUT`: an empty file in another temp folder;
    - `RUNNER_TEMP`: a third, empty temp folder.
    All three are separate from each other and from the site folder. The helper returns the
    step's exit code, stdout and the `GITHUB_OUTPUT` text.
  - S1 builds the expected report from strings. The entry header is
    `post.md:1  PERMANENT REDIRECT 301 200  <w.url("/old")>`, the detail line
    `    fix: <w.url("/new")>`, then the summary line. The summary line is
    `checked 1: ok 0, permanent 1, temporary 0, dead 0, suspect 0, unverifiable 0, blocked 0, ignored 0`.
    Confirm this format against a plain `w.run(&["--apply"])` on a copy while writing the test;
    don't depend on a second run in the test itself. Then add the heading line, the empty
    line and the fences exactly as the design's Details lay them out. The comparison is byte
    for byte.
  - S2 compares `w.site.snapshot()` before and after (no file added, none changed). It checks
    that `manual fix: no archived copy` is in both the step's stdout and the report file.
  - S3 reads the generated `rotguard.toml`, appends `colour = "red"` on its own line and writes
    it back.
  - S4 and S5 compare whole strings exactly, after trimming the trailing newline of block
    scalars only where the scenario says "trimmed". S5 tests for the `workflow_dispatch` key's
    presence (its value is null). It takes the first block whose opening fence line is
    exactly three backticks followed by `yaml`.
  - Before T2 and T3, S1 to S4 fail because `action.yml` is missing, and S5 because
    `docs/github-actions.md` is missing. Each fails with a panic message naming the missing
    file.
- Covers: S1, S2, S3, S4, S5
- Commit: Add acceptance tests for the weekly fix PR

### T2: Composite action
- Files: action.yml (new)
- Interfaces: Produces `action.yml`, laid out as the design's Details: no `inputs`. Its outputs
  are `exit-code` and `report`, mapped from `steps.run.outputs`. A build step runs
  `rustup toolchain install stable --profile minimal` and
  `cargo +stable build --release --locked --manifest-path "$GITHUB_ACTION_PATH/Cargo.toml"`.
  The step with `id: run` follows; both steps have `shell: bash`. The run step does this:
  - `set +e` around `"$GITHUB_ACTION_PATH/target/release/rotguard" --apply`, with stdout to a
    file under `$RUNNER_TEMP`;
  - `cat` that file to its own stdout and print nothing else there;
  - write the report file `$RUNNER_TEMP/rotguard-report.md`, byte for byte as Details says;
  - append `exit-code=<code>` and `report=$RUNNER_TEMP/rotguard-report.md` to `$GITHUB_OUTPUT`;
  - exit 0.
- Test: un-pends S1, S2, S3, S4. Run S1 to S3 under `/bin/bash` on this Mac; they must pass
  there.
- Covers: R1, R2, R3, R4, R8, S1, S2, S3, S4
- Commit: Add a GitHub Action that runs rotguard

### T3: Site workflow doc
- Files: docs/github-actions.md (new)
- Interfaces: Consumes T2's outputs `exit-code` and `report`. Produces the doc. Its first
  `yaml` code block is the site workflow from the design's Details, with the notify title
  `rotguard: weekly run failed`. Check the current major versions of `actions/checkout` and
  `peter-evans/create-pull-request` with
  `gh api repos/actions/checkout/releases/latest --jq .tag_name` and
  `gh api repos/peter-evans/create-pull-request/releases/latest --jq .tag_name`, and pin those
  majors. If `gh` fails, keep `@v4` and `@v7`. After the block:
  - where to save it on the site: `.github/workflows/rotguard.yml`;
  - the two repo settings from Details;
  - the outcomes table;
  - the note that e2e checks on the PR may wait for an "Approve workflows to run" click;
  - drop the `notify` job on a repo without `telegram-notify.yml`; GitHub still emails the
    person who last edited the schedule when a scheduled run fails;
  - `gh workflow run rotguard.yml` for a manual run.
  About 70 lines, written as whittle's `prompts/writing.md` says.
- Test: un-pends S5.
- Covers: R5, R6, R7, S5
- Commit: Document the weekly fix workflow

### T4: Docs
- Files: docs/architecture.md, README.md
- Interfaces: none. Read the branch's diff against `Base:` in `.whittle/run/progress.md`.
  - `docs/architecture.md`: add the action as a second way to run the binary, in the Context
    and Components (`action.yml`: builds rotguard on a GitHub runner and runs `--apply`; the
    site workflow in `docs/github-actions.md` opens the PR). Add one Decisions line citing
    `docs/design/weekly-fix-pr.md` D1 and D6. Keep the Limits line "No state between runs";
    a scheduled run lives in CI, not in the binary.
  - `README.md`: one line under Use, or a short "Run weekly" paragraph, pointing to
    `docs/github-actions.md`.
- Test: none
- Covers: none
- Review: skip
- Commit: Update docs for the weekly fix PR

## Verification contract
```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'
! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'
scripts/privacy-check.sh --all
```
