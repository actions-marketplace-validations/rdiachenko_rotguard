---
status: approved
---
# Readable fix PR body: a markdown report for the weekly PR

## Overview

### Problem
The weekly `rotguard` workflow opens a PR whose body is rotguard's text report pasted into a code
block, the same lines the run log shows. In PR #45 that was one long line per
finding, a file path, a class such as `PERMANENT REDIRECT 308 200`, a URL, and an indented
`fix:` line. Nothing says which entries the PR changed and which need a hand. The fix showed up
twice because the same link sits at two lines, and the `UNVERIFIABLE tls` entry looked as
important as the fix. The user reads this body every week before merging, so it should say at a
glance what was fixed, what needs their hand, and what is only worth knowing.

### Use cases
**Review the weekly PR.** The PR from `rotguard/fixes` opens with a body like this, rendered by
GitHub (shown abbreviated; a cell holds the full path or URL):

```markdown
## Fixed: 1 link in 2 places

| Page | Lines | Old link | New link | Why |
|---|---|---|---|---|
| …/index.md | 26, 418 | <…/engineering/…> | <…/research/…> | moved permanently (308 200) |

## Needs a hand fix

Nothing.

<details><summary>1 more link to know about, no action needed</summary>

| Page | Lines | Link | What rotguard saw |
|---|---|---|---|
| content/about.md | 30 | <…youtube.com/@rdiachenko> | moved temporarily (302 303 200) to <…> |

</details>

90 links checked: 84 ok, 1 moved permanently, 1 moved temporarily, 4 blocked.
```

The links are clickable, the repeated fix is one row with both lines, and the temporary
redirect is folded away.

**A link needs a hand fix.** A dead link with no archived copy gets a row under
`## Needs a hand fix: 1 link in 1 place`, with the problem (`dead (404)`) and why rotguard
couldn't fix it (`no archived copy in the Wayback Machine`). The job still fails and the
Telegram alert still fires. The run's summary page shows the same rendered report, so the alert's
link lands on something readable even when no PR opened.

**Read it locally.** `rotguard --format markdown` prints the same report in a terminal. Without
`--apply`, the fixes sit under `## Fixes to apply` with the line ``Run `rotguard --apply` to
write them.``

### How it works
rotguard gains a fourth report format next to text, JSON and HTML. `src/report.rs` renders it
from the same findings and summary, after `--apply` has run, so it knows which fixes were
written. The action runs `rotguard --apply --format markdown` instead of the text report and
uses its stdout as the PR body as it is, with no heading or code fence around it. The log and the
job summary page show the same markdown.

```text
Context: one weekly run, on the site's runner

  site files ──► rotguard --apply --format markdown
                   │ stdout
                   ▼
                 $RUNNER_TEMP/rotguard-report.md ──┬──► job log
                                                   ├──► $GITHUB_STEP_SUMMARY (run page)
                                                   └──► output `report`
                                                          │ body-path, on exit 0 or 1
                                                          ▼
                                                        create-pull-request ──► PR body
```

**Main flow.**
1. The action's run step runs rotguard with `--apply --format markdown`, stdout to the report
   file. rotguard writes the fixes, then renders the markdown, so each fix is either written
   (`Fixed`) or not (`Needs a hand fix`).
2. The step prints the report file to the log and appends it to `$GITHUB_STEP_SUMMARY`, the
   markdown page GitHub shows on the run.
3. The site workflow passes the file as `body-path` to `create-pull-request`, as today.

**Failure flow.** rotguard exits 2 on a bad config before it prints a report, so the report
file is empty, the log and summary show nothing from rotguard (its error is on stderr, in the
log), and no PR opens. A finding whose text would break the markdown, such as a URL containing
`|` or a file name with `_`, is written as escaped text, so the table keeps its columns.

**Contract.**

```text
rotguard --format markdown        stdout: the markdown report (Details), then a newline
                                  exit code, stderr and --apply writes as with --format text
action.yml run step               report file = rotguard's stdout, byte for byte
                                  log and $GITHUB_STEP_SUMMARY (when set) get the same bytes
```

The site workflow doesn't change: same outputs, same `body-path`.

### Decisions
#### D1: The markdown is built by rotguard (user's choice)
Options: `--format markdown` in rotguard; the action turns `--format json` into markdown with
jq. Chosen: rotguard. Why: unit tests in Rust cover every row and escape, the format works
locally too, and the action stays a few lines of shell. Cost if wrong: low; the format can stay
even if the action later renders something else.

#### D2: Sections by what to do (user's choice)
Options: sections Fixed, Needs a hand fix and a folded list of the rest; only the first two;
one table of every finding. Chosen: the sections. Why: the reviewer sees what the PR changed and
what is left for them before anything else. Cost if wrong: low, a change to one function.

#### D3: Same fix at several lines is one row (assumption)
Rows with the same page, link and other cells merge into one, with the lines listed as
`26, 418`. Why: PR #45 showed the same fix twice. The heading still counts places: `1 link in
2 places`.

#### D4: The log and the run's summary page show the markdown (assumption)
Options: print the markdown to the log; also print the text report, which needs a second run of
about 60 s and 90 more requests; append the markdown to `$GITHUB_STEP_SUMMARY` too. Chosen: the
log and the summary page, from one run. Why: the Telegram alert links the run, and its summary
page renders the report even on a week with no PR. Cost if wrong: one line in `action.yml`.

#### D5: Fail safe on markdown escaping (assumption)
A plain `http(s)` URL is written as an autolink `<url>`, which GitHub makes clickable. A URL that
an autolink can't hold (a space, `<`, `>`, `|`, a backtick, a backslash or a control character)
is written as escaped text instead: exact, but not clickable. Every other text cell is
backslash-escaped. Why: a broken table hides findings; a URL that isn't clickable hides nothing.

#### D6: Two locked tests of the weekly-fix-pr design change (assumption)
That design's R3 laid the PR body out as a heading plus the text report in a code fence, and its
locked tests `action_s1_fix_is_written_and_reported` and
`action_s2_hand_fix_exits_one_and_writes_nothing` assert that layout and the `manual fix:`
wording. This design replaces R3's layout, so the build amends those two tests to the markdown
report through `whittle-gates amend`. Approving this design approves that amendment.

#### D7: The other formats stay as they are (assumption)
Text, JSON and HTML don't change, nor does the text wording of an applied fix (`fix:` for both
written and proposed; the backlog's R19 line). Markdown is the only format that tells them
apart, by section.

### Scope
In:
- `--format markdown` in `src/report.rs` and the `Format` enum, ~150 lines: the PR body and
  local reading.
- The action's run step: `--format markdown`, report file from stdout, job summary, ~10 lines
  changed.
- Docs: README `--format` line, `docs/github-actions.md` sample body, `docs/architecture.md`,
  ~25 lines.

Out: links from a page path to its file on GitHub; a PR body over GitHub's 65,536-character limit
(your reports are about 1 KB); `--strict` in the action; changes to the text, JSON or HTML
reports; a heading or intro line in the PR body other than rotguard's report; changes to the
site workflow.

Size: ~185 non-test lines in total.
Touches: `action.yml` — the run step passes `--format markdown` and writes the job summary.

### Open for you
none

## Spec

### Requirements
- R1: WHEN `--format markdown` is given THE SYSTEM SHALL print to stdout the markdown report laid
  out as in Details, followed by one newline, and nothing else.
- R2: THE SYSTEM SHALL place each reported finding in exactly one section by its fix: a written
  fix in `Fixed`, a fix not written in `Fixes to apply`, a fix that needs a hand edit in
  `Needs a hand fix`, and no fix (temporary, unverifiable, blocked) in the folded list.
- R3: WHERE at least one finding is reported THE SYSTEM SHALL print `Fixed`, `Fixes to apply` and
  the folded list only when they hold a row, and print `Needs a hand fix` always, with `Nothing.`
  when it holds no row. WHERE no finding is reported THE SYSTEM SHALL print `No broken links.`
  instead of the sections.
- R4: THE SYSTEM SHALL merge findings with the same page, the same link URL and the same other
  cells into one row whose Lines cell lists their line numbers in ascending order, joined by
  `, `. Rows keep the text report's order, by page and then by first line.
- R5: THE SYSTEM SHALL word each problem and each hand-fix reason as the tables in Details say.
- R6: THE SYSTEM SHALL write a link URL that starts with `http://` or `https://` and has no
  space, `<`, `>`, `|`, backtick, backslash or control character as `<url>`, and any other URL,
  page path or text from the site or the network with the characters
  `` \ ` * _ [ ] < > ( ) ! | ~ & # `` backslash-escaped and each control character replaced by a
  space.
- R7: THE SYSTEM SHALL end the report with the counts line described in Details, with the same
  numbers as the text summary line.
- R8: WHEN `--format markdown` is given THE SYSTEM SHALL keep the exit code, stderr and what
  `--apply` writes the same as with `--format text`.
- R9: WHEN the action runs, THE SYSTEM SHALL run
  `"$GITHUB_ACTION_PATH/target/release/rotguard" --apply --format markdown` with stdout going to
  `$RUNNER_TEMP/rotguard-report.md`, print that file to the log, append it to
  `$GITHUB_STEP_SUMMARY` when that variable is set and not empty, and keep the outputs and the
  exit 0 of the weekly-fix-pr design.

### Acceptance scenarios
M is the mock server's base URL, `http://127.0.0.1:<port>`. Each expected stdout is exact.

- S1 [R1, R2, R3, R4, R7, R8]: Given `post.md` with `[a](M/old)` on line 1 and line 3, and
  `M/old` answering 301 → `M/new` → 200, when rotguard runs with `--apply --format markdown`,
  then `post.md` links `M/new` on both lines, the exit code is 0, and stdout is:

  ```markdown
  ## Fixed: 1 link in 2 places

  | Page | Lines | Old link | New link | Why |
  |---|---|---|---|---|
  | post.md | 1, 3 | <M/old> | <M/new> | moved permanently (301 200) |

  ## Needs a hand fix

  Nothing.

  1 link checked: 0 ok, 1 moved permanently.
  ```
- S2 [R2, R3]: Given the site of S1, when rotguard runs with `--format markdown` and no
  `--apply`, then `post.md` is unchanged and stdout starts with:

  ```markdown
  ## Fixes to apply: 1 link in 2 places

  Run `rotguard --apply` to write them.

  | Page | Lines | Old link | New link | Why |
  ```
- S3 [R2, R5, R8]: Given `post.md` with `[x](M/gone)` on line 1, `M/gone` answering 404 twice
  and the archive having no capture, when rotguard runs with `--apply --format markdown`, then
  the exit code is 1, `post.md` is unchanged, and stdout is:

  ```markdown
  ## Needs a hand fix: 1 link in 1 place

  | Page | Lines | Link | Problem | Why rotguard couldn't fix it |
  |---|---|---|---|---|
  | post.md | 1 | <M/gone> | dead (404) | no archived copy in the Wayback Machine |

  1 link checked: 0 ok, 1 dead.
  ```
- S4 [R2, R3, R5]: Given `post.md` with `[t](M/t)` on line 1, and `M/t` answering 302 → `M/end`
  → 200, when rotguard runs with `--format markdown`, then the exit code is 0 and stdout is:

  ```markdown
  ## Needs a hand fix

  Nothing.

  <details><summary>1 more link to know about, no action needed</summary>

  | Page | Lines | Link | What rotguard saw |
  |---|---|---|---|
  | post.md | 1 | <M/t> | moved temporarily (302 200) to <M/end> |

  </details>

  1 link checked: 0 ok, 1 moved temporarily.
  ```
- S5 [R3, R7]: Given `post.md` whose only link `M/a` answers 200, when rotguard runs with
  `--format markdown`, then stdout is `No broken links.`, a blank line, and
  `1 link checked: 1 ok.`
- S6 [R6]: Given `a_b.md` with `[x](M/a|b)` on line 1, `M/a|b` answering 404 twice and no
  capture, when rotguard runs with `--format markdown`, then stdout has the row
  `| a\_b.md | 1 | M/a\|b | dead (404) | no archived copy in the Wayback Machine |`, with M
  written as it is (`http://127.0.0.1:<port>`, none of whose characters is escaped).
- S7 [R5]: Given `post.md` linking `M/wall/page` on line 1, every path under `M/wall/` answering
  403, when rotguard runs with `--strict --format markdown`, then stdout has the row
  `| post.md | 1 | <M/wall/page> | blocked by a bot wall (403) |` inside the `<details>` block,
  and the counts line ends `1 blocked.`
- S8 [R9]: Given `post.md` whose only line is `[x](M/old)`, `M/old` answering 301 → `M/new` →
  200, and `GITHUB_STEP_SUMMARY` set to an empty file, when the action's run step runs, then the
  step exits 0, `GITHUB_OUTPUT` has `exit-code=0` and `report=$RUNNER_TEMP/rotguard-report.md`,
  `post.md` links `M/new`, the step's stdout contains the report file's text, the
  `GITHUB_STEP_SUMMARY` file equals the report file, and the report file is exactly (with one
  trailing newline):

  ```markdown
  ## Fixed: 1 link in 1 place

  | Page | Lines | Old link | New link | Why |
  |---|---|---|---|---|
  | post.md | 1 | <M/old> | <M/new> | moved permanently (301 200) |

  ## Needs a hand fix

  Nothing.

  1 link checked: 0 ok, 1 moved permanently.
  ```
- S9 [R9]: Given the site of S8 and no `GITHUB_STEP_SUMMARY` variable in the step's environment,
  when the action's run step runs, then the step exits 0 and the report file equals S8's report.

### Details

**Layout.** Sections in this order, separated by one blank line; a section with no row is left
out, except `Needs a hand fix`. No trailing newline before `println!` adds one.

~~~text
## Fixed: {L} link|links in {P} place|places              (written fixes)

| Page | Lines | Old link | New link | Why |
|---|---|---|---|---|
| {page} | {lines} | {link url} | {fix url} | {problem} |

## Fixes to apply: {L} link|links in {P} place|places     (proposed fixes)

Run `rotguard --apply` to write them.

| Page | Lines | Old link | New link | Why |
|---|---|---|---|---|
| {page} | {lines} | {link url} | {fix url} | {problem} |

## Needs a hand fix: {L} link|links in {P} place|places   (or: ## Needs a hand fix + Nothing.)

| Page | Lines | Link | Problem | Why rotguard couldn't fix it |
|---|---|---|---|---|
| {page} | {lines} | {link url} | {problem} | {reason} |

<details><summary>{L} more link|links to know about, no action needed</summary>

| Page | Lines | Link | What rotguard saw |
|---|---|---|---|
| {page} | {lines} | {link url} | {problem} |

</details>

{checked} link|links checked: {ok} ok, {n} {label}, ….
~~~

- `{L}` is the number of distinct link URLs in the section, `{P}` the number of findings in it.
  `link` and `place` are singular for 1, plural otherwise.
- `{page}` is the path relative to the site with `/` separators, as in the text report.
- The blank lines inside `<details>` are needed for GitHub to render the table.
- With no reported finding the whole report is `No broken links.`, a blank line, the counts line.

**Problem wording.** `{statuses}` is the chain's statuses joined by spaces, as in the text report.

| Verdict | Problem cell |
|---|---|
| Permanent | `moved permanently ({statuses})` |
| Temporary | `moved temporarily ({statuses}) to {final url}`, the URL written as in R6 |
| Dead | `dead ({reason})` |
| Suspect | `suspect ({reason})` |
| Unverifiable | `could not check ({reason})` |
| Blocked (only under `--strict`, as in the text report) | `blocked by a bot wall ({status})` |

**Hand-fix reasons.**

| Reason in the text report | Reason cell |
|---|---|
| `no archived copy` | `no archived copy in the Wayback Machine` |
| `archive unreachable` | `the Wayback Machine didn't answer` |
| `deadline` | `the run ran out of time looking for an archived copy` |
| `not verbatim` | `the file writes the URL differently from the link` |
| `cannot quote` | `the new URL can't be written inside this link` |
| `changed during run` | `the file changed while rotguard ran` |
| a suspect's proposed fix (`--apply` never writes it) | `rotguard doesn't rewrite suspect links; archived copy: {fix url}`, the URL as in R6 |

**Counts line.** `{checked} link|links checked: {ok} ok`, then each nonzero count in this order:
`{permanent} moved permanently`, `{temporary} moved temporarily`, `{dead} dead`,
`{suspect} suspect`, `{unverifiable} could not check`, `{blocked} blocked`, `{ignored} ignored`,
joined by `, `, then `.`. The counts are the text summary's, which count distinct URLs.

**Escaping (R6).** It applies to page paths, statuses, reasons and every URL that isn't written
as `<url>`. The numbers, fixed wording and headings are not escaped. Cells never hold a newline.

**The action's run step** becomes, in substance:

```sh
report_file="$RUNNER_TEMP/rotguard-report.md"
set +e
"$GITHUB_ACTION_PATH/target/release/rotguard" --apply --format markdown > "$report_file"
code=$?
set -e
cat "$report_file"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then cat "$report_file" >> "$GITHUB_STEP_SUMMARY"; fi
echo "exit-code=$code" >> "$GITHUB_OUTPUT"
echo "report=$report_file" >> "$GITHUB_OUTPUT"
exit 0
```

The `mktemp` stdout file and the heading line go away. weekly-fix-pr's S4 still holds: the run
step contains `"$GITHUB_ACTION_PATH/target/release/rotguard" --apply` and no `${{`.

**Amended locked tests (D6).** In `tests/acceptance/action.rs`:
- `action_s1_fix_is_written_and_reported`: its site is S8's. The whole-report `assert_eq!` now
  expects S8's report text exactly, with `{old}` and `{new}` for `M/old` and `M/new`, instead of
  the heading and text block. `HEADING` and the `stdout` variable go.
- `action_s2_hand_fix_exits_one_and_writes_nothing`: `manual fix: no archived copy` becomes
  `no archived copy in the Wayback Machine`, in both the step's stdout and the report.
Nothing else in those tests changes. S8 and S9 are new tests in a new file with its own
run-step helper, which sets `GITHUB_STEP_SUMMARY` for S8 and removes it for S9.

**Errors.** No new I/O and no new failure path. `--format` rejects unknown values as before; the
error now also names `markdown`.

### Risk paths
`action.yml`, already in `.whittle/gates.toml`.

### Fitness rules
Unchanged: only `src/apply.rs` writes files; tests use no real-network URLs;
`scripts/privacy-check.sh --all`.

### Review changes
- Round 1: S8 and S9 get their own one-line site and S8 states the exact report file, which the
  amended `action_s1` also expects (S1's two-line site gave `2 places`).
- Branch review: a suspect's proposed fix goes to `Needs a hand fix` with its own reason row,
  since `--apply` never writes it (it showed under `Fixes to apply` with `Nothing.` to fix).
- Round 1: parked to the backlog: after an apply error (exit 2) the report still says
  ``Run `rotguard --apply` `` for the fixes it couldn't write.
