---
status: approved
---
# A fix `--apply` couldn't write is a hand fix

## Overview

### Problem
`rotguard --apply` writes each file through `<file>.rotguard.tmp` and renames it into place.
When reading, writing or renaming one file fails (a leftover `post.md.rotguard.tmp`, a
read-only folder, a full disk), that file stays as it was, stderr names the error, and rotguard
exits 2. But the report still treats that file's fixes as proposed and not yet written. The
markdown report lists them under `## Fixes to apply` with ``Run `rotguard --apply` to write
them.``, which is wrong advice right after `--apply` ran. The text report prints `fix: <url>`,
the same line as a fix that was written. On exit 2 the weekly workflow opens no PR, so this
wording lands on the run's summary page, which the Telegram alert links to, and in the run log.
No run has hit it yet.

### Use cases
**The weekly run can't write one file.** `post.md` and `a.md` both link `M/old`, which moved
permanently to `M/new`, and a `post.md.rotguard.tmp` was left behind. The run writes `a.md`,
leaves `post.md` alone, exits 2, and its summary page shows:

```markdown
## Fixed: 1 link in 1 place

| Page | Lines | Old link | New link | Why |
|---|---|---|---|---|
| a.md | 1 | <M/old> | <M/new> | moved permanently (301 200) |

## Needs a hand fix: 1 link in 1 place

| Page | Lines | Link | Problem | Why rotguard couldn't fix it |
|---|---|---|---|---|
| post.md | 1 | <M/old> | moved permanently (301 200) | rotguard couldn't write the file; its error message says why |

1 link checked: 0 ok, 1 moved permanently.
```

stderr still says `rotguard: applying fixes: post.md: post.md.rotguard.tmp already exists;
remove it and run again`.

**Running it locally.** `rotguard --apply` in a terminal prints the `post.md` entry with
`    manual fix: write failed` instead of `    fix: M/new`, and the JSON report gives that
finding `"fix": {"kind": "manual", "reason": "write failed"}`.

### How it works
`apply::apply` already returns one `ApplyError` per file it couldn't write. The change is one
step in it: when a file fails, every finding of that file whose fix is still
`Fix::Proposed` becomes `Fix::Manual(ManualReason::WriteFailed)`. The reports already render
`Fix::Manual` everywhere, so the four formats follow with one new reason string each, and no
renderer needs to know whether `--apply` ran.

```text
Context: one rotguard --apply run

  findings (Fix::Proposed) ──► apply::apply ──per file──► apply_file
                                    │                         │ Ok: Fix::Applied
                                    │                         │ Err(io): file left as it was
                                    │◄────────────────────────┘
                                    │ failed file's Proposed → Manual(WriteFailed)  (new)
                                    ▼
                               Vec<ApplyError> ──► stderr, exit 2 (unchanged)
                                    │
                                    ▼
                    report::render_as ──► Fixed / Needs a hand fix  (markdown)
                                          manual fix: write failed  (text, HTML)
                                          {"kind": "manual", ...}   (JSON)
```

**Main flow.**
1. `apply` groups the writable fixes by file and calls `apply_file` for each, as today.
2. A file that writes cleanly turns its fixes into `Fix::Applied`, as today.
3. A file whose `apply_file` returns an I/O error is left as it was. `apply` records the
   `ApplyError`, as today, and now also sets each of that file's findings that still holds
   `Fix::Proposed` to `Fix::Manual(ManualReason::WriteFailed)`.
4. The report renders the findings; stderr lists each error; the exit code is 2, as today.

**Failure flow.** A fix `apply_file` already turned into `Manual(Unwritable)` or
`Manual(ChangedDuringRun)` keeps its reason: only fixes still `Proposed` change. A file that
was deleted or became non-UTF-8 is still `changed during run` and not an error.

**Contract.**

```text
enum ManualReason { Archive(NoFix), Unwritable(Unwritable), ChangedDuringRun, WriteFailed }
ManualReason::WriteFailed displays as "write failed"            // text, HTML, JSON reason
md_reason(WriteFailed) = "rotguard couldn't write the file; its error message says why"
apply::apply(site_dir, &mut findings) -> Vec<ApplyError>        // signature unchanged
```

### Decisions
#### D1: Mark the fixes in `apply`, not tell the renderers that `--apply` ran (assumption)
Options: pass an `applied: bool` (or the apply errors) to `render_markdown` and word the
section from it; turn the failed file's fixes into `Fix::Manual` in `apply`. Chosen: the second.
Why: a finding then says what happened to its fix in every format at once, which settles both
backlog lines, and the renderers keep their signatures. Cost if wrong: low; the reason string is
the one place to change.

#### D2: The reason doesn't carry the I/O error (assumption)
`ManualReason` is a `Copy` enum of fixed strings, and the error is per file, not per link.
stderr already prints it with the file name (`post.md.rotguard.tmp already exists; remove it and
run again`), so the report points there instead of repeating it in each row.

#### D3: This settles two backlog ideas
The `[idea]` lines on R19's detail line for an unwritten fix and on the markdown report's advice
after a failed `--apply` are both settled by D1, and both leave the backlog.

### Scope
In:
- `ManualReason::WriteFailed`, its `Display`, and the marking step in `apply::apply`, ~12 lines
  in `src/lib.rs` and `src/apply.rs`: any run where a file can't be written.
- The markdown reason in `md_reason`, ~1 line in `src/report.rs`.
- Docs: the README's `manual fix:` reasons, `docs/architecture.md`'s Apply paragraph, ~4 lines.

Out: retrying a failed write; removing a leftover `.rotguard.tmp` (rotguard never deletes a file
it didn't create in that run); putting the I/O error text in the report; opening the PR on exit
2; changes to the exit code or stderr.

Size: ~17 non-test lines in total.
Touches: `src/apply.rs` — `apply` marks a failed file's proposed fixes as `WriteFailed`.

### Open for you
none

## Spec

### Requirements
- R1: IF `--apply` can't read, write or rename a file because of an I/O error THEN THE SYSTEM
  SHALL leave that file as it was and report each of its fixes not already marked manual as
  `Fix::Manual(ManualReason::WriteFailed)`.
- R2: THE SYSTEM SHALL word `WriteFailed` as `write failed` in the text, HTML and JSON reports,
  and as `rotguard couldn't write the file; its error message says why` in the markdown
  report's `Needs a hand fix` table.
- R3: WHEN a file can't be written THE SYSTEM SHALL keep writing the other files, keep printing
  `rotguard: applying fixes: <error>` to stderr for each failed file, and exit 2, as today.

### Acceptance scenarios
M is the mock server's base URL, `http://127.0.0.1:<port>`. `M/old` answers 301 → `M/new`,
which answers 200.

- S1 [R1, R2, R3]: Given `a.md` holding `[a](M/old)` and a newline, `post.md` holding
  `[b](M/old)` and a newline, and a regular file `post.md.rotguard.tmp` holding `x`, when
  rotguard runs with `--apply --format markdown`, then the exit code is 2, `a.md` holds
  `[a](M/new)` and a newline, `post.md` and `post.md.rotguard.tmp` are unchanged, stderr contains
  `rotguard: applying fixes: post.md` and `remove it and run again`, and stdout is exactly:

  ```markdown
  ## Fixed: 1 link in 1 place

  | Page | Lines | Old link | New link | Why |
  |---|---|---|---|---|
  | a.md | 1 | <M/old> | <M/new> | moved permanently (301 200) |

  ## Needs a hand fix: 1 link in 1 place

  | Page | Lines | Link | Problem | Why rotguard couldn't fix it |
  |---|---|---|---|---|
  | post.md | 1 | <M/old> | moved permanently (301 200) | rotguard couldn't write the file; its error message says why |

  1 link checked: 0 ok, 1 moved permanently.
  ```

  followed by one newline.
- S2 [R1, R2, R3]: Given the site of S1, when rotguard runs with `--apply` (text), then the exit
  code is 2, the `post.md:1` entry's detail line is `    manual fix: write failed`, and no
  `post.md` entry has a `fix:` line.
- S3 [R2]: Given the site of S1, when rotguard runs with `--apply --format json`, then the
  finding for `post.md` line 1 has `"fix": {"kind": "manual", "reason": "write failed"}` and the
  finding for `a.md` line 1 has `"fix": {"kind": "applied", "url": "M/new"}`.

### Details
- In `apply::apply`, after `apply_file` returns `Err` for a file, walk that file's indices and
  set every finding whose fix is `Fix::Proposed` to `Fix::Manual(ManualReason::WriteFailed)`.
  `indices` is moved into `apply_file` today, so keep a copy (or pass a slice). A finding
  `apply_file` already set to `Manual(Unwritable)` stays as it is.
- The doc comment on `apply` changes from "its findings keep `Fix::Proposed`" to "its proposed
  fixes become `Manual(WriteFailed)`".
- `ManualReason`'s `Display` gains `Self::WriteFailed => f.write_str("write failed")`; the table
  test `manual_reasons_print_their_report_strings` in `src/lib.rs` gains that row.
- `md_reason` gains `ManualReason::WriteFailed => "rotguard couldn't write the file; its error
  message says why"`.
- `exit_code` needs no change: `run` returns 2 whenever `apply` returned errors.
- The HTML report renders `Detail::Manual` from `Display`, so it shows `manual fix: write failed`
  with no change of its own.
- A leftover `post.md.rotguard.tmp` makes `write_atomic` fail with `AlreadyExists`, which is
  what the scenarios use; no test needs a read-only folder or a full disk.
- The unit test `one_unreadable_file_does_not_stop_the_others` (src/apply.rs:575-578) asserts
  that the file it can't read (a directory named `a.md`) keeps `Fix::Proposed`. Change that
  assertion to `Fix::Manual(ManualReason::WriteFailed)`, so it proves R1's "can't read" case;
  S1 covers the write case.

### Risk paths
`src/apply.rs`, already in `.whittle/gates.toml`.

### Fitness rules
Unchanged: only `src/apply.rs` writes files; tests use no real-network URLs;
`scripts/privacy-check.sh --all`.

### Review changes
- Round 1: Details names the unit test `one_unreadable_file_does_not_stop_the_others`, whose
  `Fix::Proposed` assertion becomes `Manual(WriteFailed)` and proves the "can't read" case.
- Round 1: Problem says a read-only folder, not a read-only file, makes a write fail (rotguard
  renames a new file over the old one).
