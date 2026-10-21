---
status: approved
---
# Cache the action's build

## Overview

### Problem
Every weekly run builds rotguard from scratch on the site's runner. In run 39498081343 the
`rotguard` step took 3 min 50 s: 19 s to install the toolchain, 2 min 8 s to compile 136
dependencies, 14 s to compile rotguard itself, and about 70 s to check the links. The
dependencies only change when `Cargo.lock` does, so most of that compile is the same work every
week. Release binaries would remove the build altogether, but they wait until rotguard is public.

### Use cases
**A normal week.** The run restores last week's `target` folder and cargo's downloaded crates from
the GitHub Actions cache, so `cargo build` compiles rotguard alone (about 14 s, if its source
changed) instead of 137 crates. The step should take roughly 2 min instead of 3 min 50 s; I'll
know the real number after the second run.

**`Cargo.lock` or the stable compiler changed.** The cache key no longer matches, so the run
starts from the newest older entry for the same OS and compiler if there is one, or from
nothing. It builds in full, as today, and saves a new entry.

**The cache is gone.** GitHub drops entries no one used for 7 days, and a weekly run sits right
at that edge. A run that finds nothing builds in full, as today, and the job is no slower than
now.

### How it works
`action.yml` gains a key step and a cache-restore step before the build, and a cache-save step
after the run step. The build and run steps don't change.

```text
Context: action.yml on the site's runner

  1 Pick cache key ──key=rotguard-<os>-<rustc>-<lock hash>──┐
    (rustup install, rustc --version,                       │
     sha256sum Cargo.lock)                                  ▼
                                               2 actions/cache/restore (id: cache)
  ┌───────────────────────────┐   restore                 │ target/, ~/.cargo/…
  │ GitHub Actions cache      ├──────────────────────────►│
  │ (site repo, 10 GB)        │                           ▼
  └─────────────▲─────────────┘                3 Build rotguard (unchanged)
                │                                         │
                │ save, unless the                        ▼
                │ exact key was restored       4 Run rotguard --apply (unchanged, exit 0)
                │                                         │
                │                                         ▼
                └──────────────────────────────5 actions/cache/save
```

**Main flow.**
1. The key step installs the stable toolchain (as the build step does today) and writes the
   outputs `rustc=<rustc version>` and
   `key=rotguard-<RUNNER_OS>-<rustc version>-<first 16 hex digits of sha256(Cargo.lock)>`.
2. `actions/cache/restore` restores the entry with that key. On a miss it restores the newest
   entry whose key starts with `rotguard-<RUNNER_OS>-<rustc version>-`, if any.
3. The build step runs as today. Cargo reuses every artifact it finds and compiles the rest.
4. The run step runs as today and always ends with exit 0.
5. Unless the exact key was restored, `actions/cache/save` saves the folders under the key. It
   runs inside the action, before the site workflow's step that fails the job on a hand fix, so a
   failing week still saves.

**Failure flow.** A cache miss, an evicted entry or a cache service error is never fatal: the
restore step warns and the build starts from nothing, as it does today. A failed build fails the
job at the build step, as today, and the save step is skipped, so a broken build is never saved.

**Contract.**

```text
step id: key   (shell: bash)   outputs: rustc, key
step id: cache   uses: actions/cache/restore@v6
  path: ${{ github.action_path }}/target, ~/.cargo/registry/index, ~/.cargo/registry/cache,
        ~/.cargo/git/db
  key: ${{ steps.key.outputs.key }}
  restore-keys: rotguard-${{ runner.os }}-${{ steps.key.outputs.rustc }}-
step: Build rotguard (unchanged)
step id: run (unchanged)
step uses: actions/cache/save@v6   if: steps.cache.outputs.cache-hit != 'true'
  path: the same four lines;   key: ${{ steps.key.outputs.key }}
```

The action's outputs and the site workflow don't change.

### Decisions
#### D1: Key on the OS, the compiler and `Cargo.lock`, not on rotguard's source (assumption)
Options: also hash `src/`; key on the lock file and compiler only. Chosen: the lock file and
compiler. Why: the dependencies are 2 min 8 s of the build and change only with `Cargo.lock`;
rotguard's own 14 s rebuilds whenever its source is newer than the cached artifacts. A compiler
update makes old artifacts useless, so it starts a new key. Cost if wrong: low; one more file in
the hash.

#### D2: The key step installs the toolchain too (assumption)
The key needs `rustc +stable --version`, so the key step runs
`rustup toolchain install stable --profile minimal`. The build step keeps that line, which then
takes about a second, so the build step stays exactly as weekly-fix-pr's locked S4 test expects.

#### D3: Save inside the action, after the run step (assumption)
Options: `actions/cache`, whose post step saves only when the whole job succeeds; separate
`restore` and `save` steps. Chosen: separate steps, with the save after `id: run`. Why: the site
job fails on every week with a hand fix, so a post-step save would never happen for a site with
a lasting dead link. Placing the save after the run step, not between the build and the run,
keeps the build step right before `id: run`, as the locked S4 test checks.

#### D6: Accept the 7-day eviction edge (assumption)
GitHub removes cache entries not used for 7 days, and the cron is 7 days apart. Restoring an
entry counts as using it, so each Monday run should keep the next one alive, but I can't prove
it ahead of time. Cost if wrong: no saving on some weeks, never a slower or failed job.

#### D4: One locked test changes (assumption)
weekly-fix-pr's `action_s4_action_yml_has_the_agreed_shape` asserts that every step has
`shell: bash` and a `run:` script. A `uses:` step has neither, so the build amends that test to
skip steps with `uses:`, and nothing else. Approving this design approves that amendment.

#### D5: This settles half of the backlog's release-binary line
The `[idea]` line keeps its release-binary part for when rotguard is public; its "until then,
`actions/cache`" clause is done and leaves the line.

### Scope
In:
- `action.yml`: the key step, the restore step and the save step, ~22 lines: every weekly run.
- Docs: `docs/github-actions.md` (the run restores a build cache), `docs/architecture.md` (the
  action's paragraph), ~6 lines.

Out: release binaries and tags (they wait for a public repo); `Swatinem/rust-cache` or another
third-party cache action; `sccache`; caching the toolchain itself (19 s); changes to the site
workflow.

Size: ~28 non-test lines in total.
Touches: `action.yml` — the key, restore and save steps.

### Open for you
none

## Spec

### Requirements
- R1: WHEN the action runs THE SYSTEM SHALL first run a step with `id: key` and `shell: bash`
  that runs `rustup toolchain install stable --profile minimal` and writes to `$GITHUB_OUTPUT`
  the lines `rustc=<V>` and `key=rotguard-<RUNNER_OS>-<V>-<H>`, where `<V>` is the second word of
  `rustc +stable --version` and `<H>` is the first 16 hex digits of the SHA-256 of
  `$GITHUB_ACTION_PATH/Cargo.lock`.
- R2: WHEN the key step has run THE SYSTEM SHALL run a step with `id: cache` that uses
  `actions/cache/restore@v6` with `key: ${{ steps.key.outputs.key }}`,
  `restore-keys: rotguard-${{ runner.os }}-${{ steps.key.outputs.rustc }}-` and the four paths
  of the contract, one per line.
- R3: THE SYSTEM SHALL keep the build step and the run step exactly as the weekly-fix-pr and
  readable-pr-body designs define them, in that order, right after the restore step.
- R4: WHEN the run step has run THE SYSTEM SHALL run, as the last step, `actions/cache/save@v6`
  with `if: steps.cache.outputs.cache-hit != 'true'`, `key: ${{ steps.key.outputs.key }}` and
  the same four paths.

### Acceptance scenarios
- S1 [R1, R2, R3, R4]: Given `action.yml`, when it is parsed as YAML, then `runs.steps` is, in
  this order:
  - the step with `id: key` and `shell: bash`;
  - the step with `id: cache`, whose `uses` is `actions/cache/restore@v6`, whose `with.key` is
    `${{ steps.key.outputs.key }}`, whose `with.restore-keys` is
    `rotguard-${{ runner.os }}-${{ steps.key.outputs.rustc }}-`, and whose `with.path` lines
    are exactly `${{ github.action_path }}/target`, `~/.cargo/registry/index`,
    `~/.cargo/registry/cache` and `~/.cargo/git/db`;
  - the build step (the one holding `cargo +stable build`);
  - the step with `id: run`;
  - a last step whose `uses` is `actions/cache/save@v6`, whose `if` is
    `steps.cache.outputs.cache-hit != 'true'`, whose `with.key` is
    `${{ steps.key.outputs.key }}`, and whose `with.path` lines equal the restore step's.

  No `run:` script contains `${{`.
- S2 [R1]: Given a temp folder as `GITHUB_ACTION_PATH` holding `Cargo.lock` with the content
  `x` and a newline, `RUNNER_OS=Linux`, an empty `GITHUB_OUTPUT` file, and a `PATH` whose first
  folder holds stub `rustup` (exits 0) and `rustc` (prints `rustc 1.95.0 (0123abcd 2026-09-15)`)
  scripts, when the key step's script runs under `bash --noprofile --norc -eo pipefail`, then
  it exits 0 and `GITHUB_OUTPUT` holds the lines `rustc=1.95.0` and
  `key=rotguard-Linux-1.95.0-73cb3858a687a849`.

### Details
- The key step's script, in substance:

  ```sh
  rustup toolchain install stable --profile minimal
  rustc_version=$(rustc +stable --version | cut -d' ' -f2)
  lock_hash=$(sha256sum "$GITHUB_ACTION_PATH/Cargo.lock" | cut -c1-16)
  echo "rustc=$rustc_version" >> "$GITHUB_OUTPUT"
  echo "key=rotguard-$RUNNER_OS-$rustc_version-$lock_hash" >> "$GITHUB_OUTPUT"
  ```

  It writes `rustc` as a second output so `restore-keys` can be built from it.
- `73cb3858a687a849` is the first 16 hex digits of the SHA-256 of `x\n` (checked with both
  `sha256sum` and `shasum -a 256`).
- The `with:` paths take `${{ github.action_path }}`, which GitHub sets only in composite
  actions; it is the same folder as `$GITHUB_ACTION_PATH`.
- **Amended locked test (D4).** In `tests/acceptance/action.rs`,
  `action_s4_action_yml_has_the_agreed_shape`'s loop over every step skips a step that has a
  `uses` key (the restore and save steps). The other assertions, including "the step before
  `id: run` holds the rustup and cargo build lines", stay as they are.
- S1 and S2 are new tests in a new file with its own helpers (the ones in `action.rs` are
  private), registered in `tests/acceptance/main.rs`.
- Errors: the key step fails the job if `rustup` or `sha256sum` fails, the same as the build
  step failing today. The restore step never fails the job on a miss, and the save step's
  default `success()` condition skips it after a failed build.

### Risk paths
`action.yml`, already in `.whittle/gates.toml`.

### Fitness rules
Unchanged: only `src/apply.rs` writes files; tests use no real-network URLs;
`scripts/privacy-check.sh --all`.

### Review changes
- Round 1: `actions/cache` became `actions/cache/restore` before the build and
  `actions/cache/save` after the run step (R4, D3), since `actions/cache` saves only when the
  whole job succeeds and the site job fails on every hand-fix week.
