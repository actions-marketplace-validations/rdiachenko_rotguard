---
status: approved
superseded-by: docs/design/readable-pr-body.md
---
# Weekly fix PR: run `rotguard --apply` on a schedule and open a PR

## Overview

### Problem
rotguard finds rotted links only when someone runs it, and on rdiachenko.com that is whenever
the user remembers. Links rot between runs: today's run on the site finds a permanent redirect
on anthropic.com that `--apply` could fix in one go, at two places in one post. Nothing runs
rotguard on a schedule, and both repos are private, so a CI job on the site can't simply
download it. The user wants a weekly job that writes the safe fixes and hands them over as a
pull request to review, and that says so when a link needs a hand fix.

### Use cases
**A week with a fix.** On Monday at 06:00 UTC the site's `rotguard` workflow runs and opens a
PR titled `Fix rotted links` from the branch `rotguard/fixes`. Its diff on today's site:

```text
content/posts/ai/end-to-end-evals-for-a-claude-code-plugin/index.md
-  …(https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents)…
+  …(https://www.anthropic.com/research/demystifying-evals-for-ai-agents)…
   (the same change at lines 26 and 418)
```

Its body is the report, inside a ```` ```text ```` block:

```text
Fixes written by `rotguard --apply`. Entries marked `manual fix:` need a hand edit.

content/about.md:30  TEMPORARY REDIRECT 302 303 200  https://www.youtube.com/@rdiachenko
    final: https://www.youtube.com/@rdiachenko?cbrd=1&ucbcb=1
content/posts/ai/end-to-end-evals-for-a-claude-code-plugin/index.md:26  PERMANENT REDIRECT 301 …
    fix: https://www.anthropic.com/research/demystifying-evals-for-ai-agents
…
```

The run passes. The user merges the PR, then runs `deploy-site.yml` as today: merging alone
doesn't deploy.

**A quiet week.** Nothing to write, so the run passes and no PR opens. If last week's PR is
still open and `main` now has the same links (fixed by hand, say), the run closes it.

**A link needs a hand fix.** A dead link has no archived copy, or its fix sits in YAML front
matter. rotguard exits 1 after `--apply`, so the PR still opens for whatever it could write,
and then the job fails. The site's `telegram-notify.yml` sends the usual failure alert with a
link to the run, whose log holds the report. It alerts every week until the link is fixed or
added to `ignore`.

**Run it now.** `gh workflow run rotguard.yml -R rdiachenko/rdiachenko.github.com` runs the
same job outside the schedule.

### How it works
rotguard itself doesn't change. This adds a GitHub Action, `action.yml` at the rotguard repo's
root, and documents the site workflow that calls it in `docs/github-actions.md`. The
architecture doc gains the action as a second way to run the binary; its Limits line "no
scheduled runs" stays true of the binary.

```text
Context: the weekly run
  ┌──────────────┐ reviews and merges PR,       ┌───────────────────────────────┐
  │ Site owner   │◄─runs deploy-site.yml by hand┤ GitHub: rdiachenko.github.com │
  └──────▲───────┘                              │ PR from rotguard/fixes        │
         │ Telegram failure alert               └───────────────▲───────────────┘
         │                                                      │ 4 push branch, open/update/
  ┌ ─ ─ ─│─ ─ ─ ─ ─ site runner: rotguard.yml (cron 0 6 * * 1)─ ┼ ─ close PR (GITHUB_TOKEN)─ ┐
    ┌────┴──────────────────┐  ┌───────────────────────────┐  ┌┴───────────────────────┐
  │ │ notify job            │  │ uses: rdiachenko/rotguard │  │ create-pull-request    │  │
    │ telegram-notify.yml   │  │ @main  [composite action] ├─►│ [peter-evans action]   │
  │ └───────▲───────────────┘  └──▲────────────┬───────────┘  └────────────────────────┘  │
     5 job failed (exit-code≠0)   │1 fetch     │ 2 HEAD/GET links, 3 CDX lookups
  └ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┼ ─ ─ action ┼ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┘
                       ┌──────────┴─────────┐  ▼
                       │ GitHub: rotguard   │  linked websites, Wayback Machine
                       │ (private, source)  │
                       └────────────────────┘
```

The runner fetches the action's repo at `@main`, source included, because the site workflow
`uses:` it. rotguard's Actions setting "Accessible from repositories owned by the user" lets the
private site repo do that without a token. The action builds the binary and runs it on the
site's checkout. Then `create-pull-request` turns whatever changed in the checkout into the PR.

**Main flow, a week with a fix.**
1. `actions/checkout` checks out the site's `main`.
2. The action's build step installs the stable toolchain and runs `cargo +stable build
   --release --locked` in the action's own folder, outside the site checkout.
3. Its run step runs `rotguard --apply` on the checkout. rotguard rewrites the two links and
   exits 0. The step prints the report to the run log, writes the PR body to
   `$RUNNER_TEMP/rotguard-report.md` and sets the outputs `exit-code=0` and `report=<that path>`.
4. `create-pull-request` sees a changed file, force-pushes `rotguard/fixes` as one commit on
   top of `main`, and opens the PR, or updates it if it is already open.
5. The exit code is 0, so the job passes and the notify job is skipped.

**Failure flow, rotguard can't do everything.**
- Exit 1 (a hand fix is left): steps 4 and 5 run as above. Then a step fails the job with
  `rotguard exited 1: a link needs a hand fix, see the Run rotguard --apply step`, and the
  notify job calls `telegram-notify.yml`.
- Exit 2 (rotguard failed, for example on a bad `rotguard.toml`), or any other code (101 on a
  panic, 128+n when killed): `--apply` may have written some files and not others, so no PR
  step runs. The job fails with `rotguard failed (exit <code>), see the Run rotguard --apply
  step`, and the alert fires.
- The build fails or the action can't be fetched: the job fails at that step, and the alert
  fires.

**Contract between the action and the site workflow.**

```text
action.yml (composite, no inputs; runs in $GITHUB_WORKSPACE)
  outputs:
    exit-code   rotguard's exit code: "0" or "1" mean the fixes are complete;
                "2" or anything else means a failed run (no PR)
    report      absolute path of the PR body file, under $RUNNER_TEMP
  log:          rotguard's report (stdout) and progress (stderr)
  side effects: rotguard's fixes in the workspace; the build in $GITHUB_ACTION_PATH/target
  never fails because of rotguard's exit code; fails only if the build fails
```

### Decisions
#### D1: A composite action in rotguard builds from its own source (user's choice)
Options: the site workflow clones rotguard with a deploy-key secret and builds it; a composite
`action.yml` in rotguard that builds from the source the runner already fetched; a reusable
workflow in rotguard that wraps the action and also opens the PR. Chosen: the action. Why: no
secret, and the build logic sits next to the code, where whittle's gates check it. Cost if
wrong: GitHub's docs don't say outright that the runner fetches the whole repo for an action.
If it doesn't, the first manual run fails at the build with "could not find `Cargo.toml`", and
switching to the deploy-key clone costs about an hour.

#### D2: A hand fix fails the job (user's choice)
Options: fail the job and let the existing Telegram alert fire; list hand fixes only in the PR;
open a GitHub issue. Chosen: fail the job. Why: no new place to watch. Cost if wrong: an alert
every Monday until the link is fixed or ignored. Moving to an issue later is a change to the
site workflow only.

#### D3: The site workflow ships as a doc in rotguard (user's choice)
Options: document the workflow in rotguard, or have the build also write it into the site repo.
Chosen: the doc. Why: the build writes only to the repo it gates. After the merge, Claude
offers to copy it into the site repo as an uncommitted change. Cost if wrong: one copy step.

#### D4: The site's `GITHUB_TOKEN` opens the PR (assumption)
Options: `GITHUB_TOKEN`, a fine-grained PAT, a GitHub App token. Chosen: `GITHUB_TOKEN`, with
the site setting "Allow GitHub Actions to create and approve pull requests" on. Why: no secret
that expires. GitHub may hold `e2e-tests.yml` on such a PR until someone with write access
clicks "Approve workflows to run". Cost if wrong: a PAT secret and one `token:` line.

#### D5: One PR on a fixed branch, rebuilt each run (assumption)
Chosen: branch `rotguard/fixes`, rebuilt from `main` and force-pushed each run, so the open PR
is updated rather than duplicated, and history stays linear for the site's
`required_linear_history`. A run with no change deletes the branch, which closes the PR.

#### D6: Only exit 0 or 1 opens a PR (assumption)
On exit 2, a panic (101) or a kill (128+n), `--apply` can stop with some files written and
others not. A PR of half the fixes would look complete. So any code but 0 or 1 only fails the
job.

#### D7: `@main`, no cache, Monday 06:00 UTC (assumption)
The action is used at `@main` until rotguard has tags. The build isn't cached: once a week it
costs a few runner-minutes of the free quota (not measured). Cron `0 6 * * 1`, plus manual
dispatch.

#### D8: `action.yml` becomes a risk path (assumption)
It runs in CI on every scheduled run, so a later change to it escalates at the merge like
`.github/**` does. This design's build is approved to create it (see Touches).

#### D9: The action has no inputs (assumption)
It always runs `rotguard --apply` on the workspace root, the user's only case. `--strict`, a
site subfolder or extra flags wait until a site needs them.

### Scope
In:
- `action.yml`: the build step and the run step, ~35 lines: the site's weekly run.
- `docs/github-actions.md`: the site workflow, the two repo settings and what each outcome
  looks like, ~70 lines: copied into the site.
- README pointer and `docs/architecture.md` updates, ~15 lines.

Out: release binaries and tags; caching the build; a PAT or GitHub App token; a reusable
workflow; a notification when a PR opens; an issue for hand fixes; `--strict` in CI; action
inputs; any edit to the site repo during the build; the backlog's run-state idea (flaky hosts).
A runner's DNS failure is still `DEAD` today (backlog line on EAI_AGAIN), so a PR can propose a
snapshot for a live link; the user reviews every PR, so nothing merges unseen.

Size: ~120 non-test lines.
Touches: `action.yml` — new file, made a risk path by this design (D8).

### Open for you
- The first real run needs your hands: commit the copied workflow on the site, turn on the two
  settings in `docs/github-actions.md`, then `gh workflow run rotguard.yml`. Recommended: do it
  right after the merge, since it is the only check of D1's build step on a real runner.

## Spec

### Requirements
- R1: WHEN the action runs, THE SYSTEM SHALL build rotguard from the action's own source with
  `cargo +stable build --release --locked --manifest-path "$GITHUB_ACTION_PATH/Cargo.toml"` and
  then run `$GITHUB_ACTION_PATH/target/release/rotguard --apply` in the caller's workspace.
- R2: WHEN rotguard exits, whatever its exit code, THE SYSTEM SHALL end the run step with exit 0
  and set the step output `exit-code` to rotguard's exit code.
- R3: WHEN rotguard exits, THE SYSTEM SHALL write the PR body to
  `$RUNNER_TEMP/rotguard-report.md`, laid out as in Details, and set the step output `report` to
  that path.
- R4: THE SYSTEM SHALL leave the workspace as rotguard left it: the action writes nothing there
  itself.
- R5: THE documented site workflow SHALL run on cron `0 6 * * 1` and on `workflow_dispatch`.
- R6: WHEN the action's `exit-code` is `0` or `1`, THE documented site workflow SHALL run
  `create-pull-request` on branch `rotguard/fixes` with `delete-branch: true` and the report
  file as the PR body.
- R7: IF the action's `exit-code` is not `0`, THEN THE documented site workflow SHALL fail the
  job after the PR step, with the message `a link needs a hand fix` for `1` and `rotguard failed`
  for any other code, and its `notify` job SHALL call `./.github/workflows/telegram-notify.yml`
  when the job failed.
- R8: WHEN rotguard exits, THE SYSTEM SHALL print rotguard's stdout, unchanged, to the run
  step's own stdout, so the run log holds the report.

### Acceptance scenarios
The tests run the action's `run` step as GitHub does: its `run:` text from `action.yml`, saved to
a file and run with `bash --noprofile --norc -eo pipefail <file>` in the site folder, with
`GITHUB_ACTION_PATH` set to a temp folder whose `target/release/rotguard` is a symlink to the
test build of rotguard, `GITHUB_OUTPUT` to an empty temp file and `RUNNER_TEMP` to an empty temp
folder. Sites use the acceptance harness's mock host M and archive A.

- S1 [R2, R3]: Given `post.md` whose line 1 links M's `/old`, which answers 301 to `/new`, which
  answers 200, when the run step runs, then the step exits 0, `post.md` links M's `/new`,
  `GITHUB_OUTPUT` holds the lines `exit-code=0` and `report=<RUNNER_TEMP>/rotguard-report.md`,
  and that file equals the heading line, an empty line, ```` ```text ````, rotguard's stdout of a
  plain `rotguard --apply` run on the same input (the `post.md:1  PERMANENT REDIRECT 301 200`
  entry with its `fix:` line, then the summary line), and ```` ``` ````.
- S2 [R2, R4, R8]: Given `post.md` linking M's `/gone`, which answers 404 on every try, and A
  with no captures, when the run step runs, then the step exits 0, `GITHUB_OUTPUT` holds
  `exit-code=1`, the report file and the step's stdout both hold `manual fix: no archived
  copy`, and every file in the site folder is byte-identical to before the run and no file was
  added.
- S3 [R2]: Given a `rotguard.toml` with the unknown key `colour = "red"`, when the run step runs,
  then the step exits 0 and `GITHUB_OUTPUT` holds `exit-code=2`.
- S4 [R1]: Given `action.yml`, when it is parsed as YAML, then `runs.using` is `composite`; it
  has no `inputs`; the step before the one with `id: run` runs a script containing `rustup
  toolchain install stable --profile minimal` and `cargo +stable build --release --locked
  --manifest-path "$GITHUB_ACTION_PATH/Cargo.toml"`; the run step's script calls
  `"$GITHUB_ACTION_PATH/target/release/rotguard" --apply`; `outputs.exit-code.value` is
  `${{ steps.run.outputs.exit-code }}` and `outputs.report.value` is
  `${{ steps.run.outputs.report }}`; every step has `shell: bash`; and no step's `run` text
  contains `${{`.
- S5 [R5, R6, R7]: Given `docs/github-actions.md`, when its first ```` ```yaml ```` block is
  parsed, then `on.schedule[0].cron` is `0 6 * * 1` and `on` has `workflow_dispatch`; job `fix`
  has a step with `id: rotguard` and `uses: rdiachenko/rotguard@main`; a later step's `uses`
  starts with `peter-evans/create-pull-request@`, its `if` is
  `steps.rotguard.outputs.exit-code == '0' || steps.rotguard.outputs.exit-code == '1'`, and its `with` has `branch: rotguard/fixes`,
  `delete-branch: true` and `body-path: ${{ steps.rotguard.outputs.report }}`; a step after that
  has `if: steps.rotguard.outputs.exit-code != '0'` and a `run` that contains `a link needs a
  hand fix` and `rotguard failed` and, trimmed, ends in `exit 1`; and job
  `notify` has `needs: [fix]`, `if: always() && contains(needs.*.result, 'failure')` and
  `uses: ./.github/workflows/telegram-notify.yml` with `with.title` `rotguard: weekly run failed`.

### Details
**`action.yml` layout.**

```yaml
name: rotguard
description: Check a markdown site's links and write the safe fixes with rotguard --apply.
outputs:
  exit-code:
    description: rotguard's exit code, 0, 1 or 2.
    value: ${{ steps.run.outputs.exit-code }}
  report:
    description: Path of a markdown file with the report, for a PR body.
    value: ${{ steps.run.outputs.report }}
runs:
  using: composite
  steps:
    - name: Build rotguard
      shell: bash
      run: |
        rustup toolchain install stable --profile minimal
        cargo +stable build --release --locked --manifest-path "$GITHUB_ACTION_PATH/Cargo.toml"
    - id: run
      name: Run rotguard --apply
      shell: bash
      run: |
        … (R2–R4; see below)
```

**The run step.** It runs with `-eo pipefail`, so it turns `-e` off around the rotguard call
only, captures the exit code, and writes the outputs. It must run under macOS's bash 3.2 too,
because the tests run it there. It uses no `${{ }}` expressions: everything it needs comes from
`GITHUB_ACTION_PATH`, `GITHUB_OUTPUT` and `RUNNER_TEMP`, so the tests run the same text GitHub
does. rotguard's stderr (progress) goes to the step's log. Its stdout goes to a temp file under
`RUNNER_TEMP`, never the workspace, and the step then prints that file to its own stdout (R8).

**The report file**, byte for byte, where `<stdout>` stands for rotguard's stdout, which ends
in a newline (an empty stdout adds nothing):

~~~text
Fixes written by `rotguard --apply`. Entries marked `manual fix:` need a hand edit.

```text
<stdout>
```
~~~

That is the heading line, an empty line, the opening fence line, the stdout, and the closing
fence line ```` ``` ```` followed by a newline. On exit 2 the file is written too (stdout is often
empty); the site workflow doesn't use it then.

**The site workflow** in `docs/github-actions.md`, saved on the site as
`.github/workflows/rotguard.yml`:

```yaml
name: rotguard
on:
  schedule:
    - cron: "0 6 * * 1"
  workflow_dispatch:
permissions:
  contents: write
  pull-requests: write
concurrency: rotguard
jobs:
  fix:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - id: rotguard
        uses: rdiachenko/rotguard@main
      - if: steps.rotguard.outputs.exit-code == '0' || steps.rotguard.outputs.exit-code == '1'
        uses: peter-evans/create-pull-request@v7
        with:
          branch: rotguard/fixes
          delete-branch: true
          title: Fix rotted links
          commit-message: Fix rotted links
          body-path: ${{ steps.rotguard.outputs.report }}
      - if: steps.rotguard.outputs.exit-code != '0'
        env:
          CODE: ${{ steps.rotguard.outputs.exit-code }}
        run: |
          if [ "$CODE" = 1 ]; then
            echo "::error::rotguard exited 1: a link needs a hand fix, see the Run rotguard --apply step"
          else
            echo "::error::rotguard failed (exit $CODE), see the Run rotguard --apply step"
          fi
          exit 1
  notify:
    needs: [fix]
    if: always() && contains(needs.*.result, 'failure')
    uses: ./.github/workflows/telegram-notify.yml
    with:
      title: "rotguard: weekly run failed"
    secrets: inherit
```

The docs task confirms the current major versions of `actions/checkout` and
`peter-evans/create-pull-request` with `gh api repos/<owner>/<repo>/releases/latest` and uses
those. S5 doesn't pin them. The doc says to drop the `notify` job on a repo without
`telegram-notify.yml`; GitHub still emails the person who last edited the schedule when a
scheduled run fails.

**The two settings** the doc lists:
1. rotguard repo: Settings → Actions → General → Access → "Accessible from repositories owned
   by the user".
2. Site repo: Settings → Actions → General → Workflow permissions → "Allow GitHub Actions to
   create and approve pull requests".

**Outcomes table.**

| rotguard exit | Workspace changed | PR step | Job | Alert |
|---|---|---|---|---|
| 0 | yes | opens or updates the PR | passes | no |
| 0 | no | closes an open PR, else nothing | passes | no |
| 1 | yes | opens or updates the PR | fails | yes |
| 1 | no | closes an open PR, else nothing | fails | yes |
| 2, or any other code | any | skipped | fails | yes |
| build or fetch fails | no | skipped | fails | yes |

**What only a real run checks.** The build step on `ubuntu-latest`, the runner fetching the
action's source (D1), both settings, the token's PR rights and the Telegram call. The tests
cover the run step and the YAML shape; the first manual run covers the rest.

**Test harness.** The tests parse YAML with `serde_norway`, already a dependency. They find the
step by `id`, never by position, except where S4 and S5 say "before" or "after". bash is
`/bin/bash` or the first `bash` on `PATH`.

### Risk paths
`action.yml`, added to `risk_paths` in `.whittle/gates.toml` (D8). The existing ones stay.

### Fitness rules
none new. The existing three still apply: only `src/apply.rs` writes files, tests never hit the
real network, no private data in tracked files.

### Review changes
- Round 1: the run step also prints the report to the run log (new R8, S2 checks it), so a hand
  fix's alert links to a log that names the link.
- Round 1: only exit 0 or 1 opens a PR (R6, S5, D6, the contract); a panic or kill is treated
  like exit 2.
- Round 1: the fail step says `a link needs a hand fix` only for exit 1 and `rotguard failed`
  otherwise (R7, S5).
- Round 2: the Telegram alert's title is `rotguard: weekly run failed` for every failure, not
  `links need a hand fix` (S5).
