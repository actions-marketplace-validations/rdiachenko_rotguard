# Plan: Cache the action's build
Design: docs/design/build-cache.md
Risk paths: action.yml — T3 adds the key, restore and save steps
Diff estimate: 30

### T1: Acceptance tests
- Files: tests/acceptance/build_cache.rs (new), tests/acceptance/main.rs
- Interfaces: none
- Test: `build_cache_s1_…` and `build_cache_s2_…`, marked `#[ignore = "pending: S1"]` and
  `"pending: S2"`, both failing against today's action.yml (it has no `id: key` step).
  - S1 parses action.yml (serde_norway, as tests/acceptance/action.rs does) and checks the five
    steps in order exactly as the design's S1 says.
  - S2 writes the `id: key` step's script to a temp file and runs it with
    `bash --noprofile --norc -eo pipefail`. Its env is:
    - `GITHUB_ACTION_PATH`: a temp folder holding `Cargo.lock` = `x\n`;
    - `RUNNER_OS=Linux`;
    - `GITHUB_OUTPUT`: an empty temp file;
    - `PATH`: a temp folder of executable (0o755) stub `rustup` and `rustc` scripts, then the
      system `PATH`.
  - S2 asserts exit 0 and that `GITHUB_OUTPUT` has the lines `rustc=1.95.0` and
    `key=rotguard-Linux-1.95.0-73cb3858a687a849`.
  - Both tests use helpers of their own, since those in action.rs are private.
  - `main.rs` is locked: open it with `whittle-gates amend` and add `mod build_cache;` only.
- Covers: S1, S2
- Commit: Add acceptance tests for the build cache

### T2: Let action_s4 skip uses steps
- Files: tests/acceptance/action.rs
- Interfaces: none
- Test: the design's D4 amendment, through `whittle-gates amend`: in
  `action_s4_action_yml_has_the_agreed_shape`, the per-step loop skips a step that has a `uses`
  key; nothing else changes. It passes before and after T3 (today action.yml has no `uses`
  step). Judged with the amendment bar and committed on its own.
- Covers: none
- Review: skip
- Commit: Amend acceptance tests: skip uses steps in S4

### T3: Cache the build in the action
- Files: action.yml
- Interfaces: Produces the design's contract: step `id: key` (outputs `rustc`, `key`), step
  `id: cache` using `actions/cache/restore@v6`, the unchanged build and run steps, then
  `actions/cache/save@v6` with `if: steps.cache.outputs.cache-hit != 'true'`.
- Test: un-pends S1 and S2; weekly-fix-pr's action_s1–s5 and readable-pr-body's S8/S9 still pass.
- Covers: R1, R2, R3, R4, S1, S2
- Commit: Cache the build in the action

### T4: Docs
- Files: docs/github-actions.md, docs/architecture.md
- Interfaces: none
- Test: none. Read the branch's diff against `Base:` in .whittle/run/progress.md.
  - docs/github-actions.md: one sentence that the action restores and saves a build cache in the
    site repo's Actions cache, so most weeks skip compiling the dependencies; keep the first yaml
    block unchanged (action_s5 parses it).
  - docs/architecture.md:59-62: the action builds rotguard on the runner from a restored build
    cache; add a Decisions line citing `docs/design/build-cache.md` D3.
- Covers: none
- Review: skip
- Commit: Update docs for the build cache

## Verification contract
```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'
! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'
scripts/privacy-check.sh --all
```
