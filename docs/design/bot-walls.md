---
status: approved
---
# Bot walls: tell a host that blocks rotguard from a forbidden page

## Overview

### Problem
rotguard reports every 401, 403 and 999 as `UNVERIFIABLE`, and on rdiachenko.com every one of
them is a false positive. Without its `ignore` list the site reports three 403s
(netflixtechblog.medium.com, stackoverflow.com, dl.acm.org) and LinkedIn's 999. All of them are
live pages behind a bot wall (Cloudflare's "Attention Required!" page). A browser User-Agent and
browser headers still get the 403, and a URL invented on the same host gets it too, so the
status says nothing about the page. The user has hidden those hosts with `ignore`, which also
hides a real removal there. A 403 still means something on a host that answers 404 for a missing
page: an S3 object, or a page made private.

### Use cases
**A normal run stays quiet about bot walls.** With the site's `ignore` list emptied, today's run
lists four `UNVERIFIABLE` entries. After this change it lists none of them, and the summary
counts them:

```text
$ rotguard
content/about.md:30  TEMPORARY REDIRECT 302 303 200  https://www.youtube.com/@rdiachenko
    final: https://www.youtube.com/@rdiachenko?cbrd=1&ucbcb=1
checked 82: ok 77, permanent 0, temporary 1, dead 0, suspect 0, unverifiable 0, blocked 4, ignored 0
```

The exit code is 0, as it is today for unverifiable links.

**`--strict` shows which links are blocked, and fails the run.**

```text
$ rotguard --strict
content/about.md:30  BLOCKED 999  https://www.linkedin.com/in/ruslan-diachenko/
…  three more BLOCKED 403 entries: the medium.com, stackoverflow.com and dl.acm.org links
…  and the YouTube temporary redirect, as today
$ echo $?
1
```

**A real 403 still shows up.** A link to a deleted S3 object gets 403, but a made-up path on
the same bucket host gets 404. So the 403 means something, and the run lists the link as it
does today: `post.md:7  UNVERIFIABLE 403  https://bucket.example/file.pdf`.

**Scripts read the count.** `rotguard --format json | jq .summary.blocked` prints `4`.
`findings` holds the blocked links only under `--strict`, with `"class": "blocked"`.

### How it works
This changes one step of the check flow in `docs/architecture.md`: classifying a fetch. The
context stays the same: rotguard, the site directory, the linked websites and the Wayback
Machine. The only new traffic is one extra GET to a linked website, and it happens only when a
link answers 401, 403 or 999. The Limits line "a site that blocks rotguard shows up as
unverifiable links" becomes true only for 429.

```text
  ┌──────────────────────────────── rotguard ────────────────────────────────┐
  │ check_one ──1 fetch──► classify ──2 401/403/999──► Probes::probe          │
  │                                                      │          ▲         │
  │ 5 same status ──► Blocked { status }                 │          │         │
  │   other status or no answer ──► Unverifiable (today) │          │         │
  │ 6 lib::run: drop Blocked unless --strict ──► report ─┼──────────┼─────────┼──► stdout
  │   summary counts every blocked URL                   │          │         │
  └──────────────────────────────────────────────────────┼──────────┼─────────┘
                       3 GET <parent path>/<random token> │          │ 4 probe's status
                                                          ▼          │
                                                 ┌──────────────────┴┐
                                                 │ Linked website    │
                                                 └───────────────────┘
```

**Main flow, a bot wall.**
1. `dl.acm.org/doi/10.1145/1273445.1273458` answers 403, and `classify` makes it
   `Unverifiable` as today.
2. The status is one of 401, 403 or 999, so `check_one` asks `Probes::probe` for the link's
   final URL.
3. The probe fetches `dl.acm.org/doi/10.1145/<random token>`.
4. It answers 403 too.
5. Same status, so the link is `Blocked { status: 403 }`.
6. The summary counts it. The report lists it only under `--strict`.

The probe is the same one the soft-404 check uses: one per origin and parent path, cached for
the run. So ten links under `/doi/10.1145/` cost one extra request.

**Failure flow, a real 403.** An S3 link answers 403 because its object was deleted. The probe
on the same bucket answers 404: a different status, so the link stays `UNVERIFIABLE 403` and is
listed as it is today. A probe that times out or fails also leaves the link `UNVERIFIABLE 403`.
A run that reaches its deadline during the probe gives `UNVERIFIABLE deadline`, as any check
cut off by the deadline does today.

**Contracts.**

```rust
// src/check.rs
pub enum Verdict { /* …existing variants… */ Blocked { status: u16 } }

// src/report.rs
pub struct Summary { /* … */ pub unverifiable: usize, pub blocked: usize, pub ignored: usize }
```

- Text entry: `<file>:<line>  BLOCKED <status>  <url>`, with no detail line.
- Summary line: `…, unverifiable U, blocked B, ignored I`.
- JSON finding: `"class": "blocked"`, `"chain": []`, `"reason": "<status>"`,
  `"final_url": null`, `"fix": null`.
- JSON summary: a new key, `blocked`.

### Decisions
#### D1: A same-status probe beside the URL decides (chosen by the user)
Options: probe a sibling; treat every 401/403/999 as blocked; detect vendor headers or titles.
Chosen: the probe. Why: it is not tied to one vendor, it keeps a real 403 visible on a host that
answers 404 for missing pages, and it reuses the soft-404 probe. Cost if wrong: low. One known
miss: a store that answers 403 for both missing and private objects (an S3 bucket without list
access) makes a deleted object look blocked, so it is listed only under `--strict`. The site has
no such link.

#### D2: Blocked links are listed only under `--strict` (chosen by the user)
Options: a summary count only; always listed; listed only under `--strict`. Chosen: `--strict`.
Why: the default run stays quiet, while `blocked N` in the summary says there is something to
see and `--strict` shows which links. Cost if wrong: low.

#### D3: Under `--strict` a blocked link fails the run, like an unverifiable one
Assumption: `--strict` means "fail on anything rotguard could not confirm", and a blocked link
is that. A CI job that runs `--strict` fails on bot-walled hosts until they are in `ignore`.
Cost if wrong: low (one match arm).

#### D4: The summary always shows `blocked N`, even when it is 0
Options: always; only when N > 0. Chosen: always, as every other count is. JSON keeps stable
keys (report-format D3). Cost: amending three locked test files by one count each.

#### D5: The probe runs whatever the host's `soft404` setting
`soft404 = false` turns off soft-404 guessing on 2xx pages. It says nothing about bot walls, and
the probe here changes no 2xx verdict.

### Scope
In:
- Blocked detection, ~35 lines in src/check.rs: on a 401, 403 or 999, fetch one probe beside
  the final URL (the soft-404 probe and its cache). The same status there means BLOCKED. All 4
  of the site's false positives.
- Report and exit code, ~30 lines in src/lib.rs and src/report.rs: blocked links are hidden
  unless `--strict` lists them (the user's choice), and the summary always counts them.
- README: the BLOCKED class, `--strict`, the summary sample (~8 lines, docs).

Out:
- Getting past bot walls (a browser User-Agent, headless browsers, cookies): tried, and none of
  the site's walls let a browser User-Agent through.
- Detecting walls by vendor headers or page titles (`cf-mitigated`, "Just a moment...").
- A per-host `blocked` config key: the probe decides, and `ignore` still exists.
- Checking blocked pages through the Wayback Machine.
- 429 and 5xx: they are retried and stay `UNVERIFIABLE`.
- Editing the user's `rotguard.toml` (its `ignore` list stays theirs to trim).

Size: ~75 non-test lines in total.
Touches: none — no file under `risk_paths` changes.

### Open for you
none

## Spec

### Requirements
- R1: WHEN a link's final response is 401, 403 or 999 THE SYSTEM SHALL fetch one probe URL: the
  final URL with its last path segment replaced by a random 24-character `[a-z0-9]` token and
  its query and fragment removed, which is the soft-404 probe's URL for that final URL.
- R2: IF the probe's final response has the same status as the link's THEN THE SYSTEM SHALL
  classify the link as blocked with that status. Otherwise (another status, or a probe that
  fails) the link stays `UNVERIFIABLE <status>`, as today.
- R3: WHERE `--strict` is not given THE SYSTEM SHALL leave blocked links out of the report
  entries (text entries, JSON `findings`, HTML rows) and out of the exit code.
- R4: WHERE `--strict` is given THE SYSTEM SHALL report every occurrence of a blocked link as
  `BLOCKED <status>`, with no detail line and no fix (`--apply` never writes one), and count it
  toward exit code 1 as it counts an unverifiable link. In JSON its finding has
  `"class": "blocked"`, `"chain": []`, `"reason": "<status>"`, `"final_url": null` and
  `"fix": null`. In HTML its Class cell reads `BLOCKED <status>`.
- R5: THE SYSTEM SHALL count distinct blocked URLs in the summary as `blocked N`, placed between
  `unverifiable` and `ignored`, with or without `--strict`. The JSON `summary` gains the key
  `blocked`. A blocked URL is not counted as `unverifiable`.
- R6: THE SYSTEM SHALL make the R1 probe whether or not the host's `soft404` is on, and at most
  one probe per (origin, parent path) in a run, shared with the soft-404 probes.

### Acceptance scenarios
M is the mock server; any path no route matches answers 404.
- S1 [R1, R2, R3, R5]: Given `post.md` with the line `[x](M/wall/page)`, and M answering 403 to
  every path under `/wall/`, when rotguard runs, then stdout has no entry, the summary line is
  `checked 1: ok 0, permanent 0, temporary 0, dead 0, suspect 0, unverifiable 0, blocked 1,
  ignored 0`, rotguard exits 0, and M received exactly one request whose path is `/wall/`
  followed by a 24-character `[a-z0-9]` token.
- S2 [R2, R5]: Given `post.md` with `[x](M/forbidden)`, M answering 403 at `/forbidden` and 404
  everywhere else, when rotguard runs, then stdout has the entry
  `post.md:1  UNVERIFIABLE 403  M/forbidden`, and the summary line is
  `checked 1: ok 0, permanent 0, temporary 0, dead 0, suspect 0, unverifiable 1, blocked 0,
  ignored 0`.
- S3 [R3, R4]: Given the site of S1, when rotguard runs with `--strict`, then stdout has exactly
  one entry, `post.md:1  BLOCKED 403  M/wall/page`, with no detail line, the summary has
  `blocked 1`, and rotguard exits 1.
- S4 [R3, R4, R5]: Given the site of S1, when rotguard runs with `--format json`, then
  `findings` is `[]` and `summary.blocked` is 1. When it runs with `--strict --format json`,
  then `findings` is `[{"file": "post.md", "line": 1, "url": M/wall/page, "class": "blocked",
  "chain": [], "reason": "403", "final_url": null, "fix": null}]`, `summary.blocked` is 1 and
  `summary.unverifiable` is 0.
- S5 [R1, R2]: Given `post.md` with `[x](M/a)`, M answering `/a` with 302 → `M/q/1`, and 999 to
  every path under `/q/`, when rotguard runs with `--strict`, then the entry for `M/a` is
  `BLOCKED 999`, and M received a request whose path is `/q/` followed by a 24-character
  `[a-z0-9]` token.
- S6 [R2]: Given `timeout_ms = 200`, `post.md` with `[x](M/w/page)`, M answering 403 at
  `/w/page` and answering every other path under `/w/` with 403 after 1 second, when rotguard
  runs, then the entry for `M/w/page` is `UNVERIFIABLE 403` and the summary has `blocked 0`.
- S7 [R6]: Given `[hosts."127.0.0.1"] soft404 = false`, and `post.md` with `[a](M/wall/a)` and
  `[b](M/wall/b)`, M answering 403 to every path under `/wall/`, when rotguard runs, then the
  summary has `blocked 2`, and M received exactly one request whose path is `/wall/` followed by
  a 24-character `[a-z0-9]` token.
- S8 [R4]: Given the site of S1, when rotguard runs with `--strict --apply`, then `post.md` is
  byte-for-byte unchanged.

### Details
- src/check.rs: `Verdict` gains `Blocked { status: u16 }`. `classify` is unchanged. In
  `check_one`, after the final `classify`: when the outcome is `Ok(fetched)`, the verdict is
  `Unverifiable`, and the last hop's status is 401, 403 or 999, call
  `probes.probe(fetcher, &last.url)`. If it returns `ProbeResult::Answered { status, .. }` with
  the same status, return `Verdict::Blocked { status }`. The probe runs inside the URL's
  deadline, like any other check. `Probes::probe` is reused as it is: it builds the sibling URL
  and caches by (origin, parent path), which gives R1 and R6. This call doesn't look at
  `host.soft404`.
- src/lib.rs: the finding's fix for `Blocked` is `Fix::None`. After the findings are built,
  `run` drops `Blocked` findings unless `args.strict` (R3). `--apply` sees the same list, so
  `Fix::None` means it writes nothing (R4). `exit_code` treats `Blocked` like `Unverifiable`
  (strict only). The summary counts `Blocked` into a new `summary.blocked`.
- src/report.rs: `Summary` gains `pub blocked: usize` after `unverifiable`, printed as
  `, blocked {blocked}` before `, ignored`, and serialised in that order. `describe` gives
  `BLOCKED <status>` with no detail. `rows` gives `("blocked", [], Some(status string), None)`;
  the row owns the reason string, or holds it in a `Cow`. The HTML report goes through
  `describe` and needs nothing new.
- README: a table row
  `| BLOCKED | 401, 403 or 999 that a made-up URL on the same host also gets: a bot wall | none; listed only with --strict |`.
  The UNVERIFIABLE row reads `a 401, 403 or 999 the host doesn't give everywhere, 429, 5xx, ...`.
  Update the `--strict` line and the exit-code paragraph, and add `blocked 0` to the summary
  sample.
- Locked acceptance tests change through `whittle-gates amend`, because R5 changes the summary
  line and the JSON summary that earlier designs fixed:
  - tests/acceptance/support.rs: `Summary` gains `blocked`, printed between `unverifiable` and
    `ignored`;
  - tests/acceptance/report.rs: the exact summary line;
  - tests/acceptance/format.rs: the summary constant and S2's `summary` object gain
    `blocked 0` / `"blocked": 0`;
  - tests/acceptance/main.rs: gains `mod blocked;`.

  Approving this design approves those amendments; the judge checks that each one only adds the
  `blocked` count. s22 and s38 (`/forbidden` → `UNVERIFIABLE 403`) stay as they are, since their
  unmatched paths answer 404.

### Risk paths
none touched; the list in `.whittle/gates.toml` is unchanged.

### Fitness rules
Unchanged: only `src/apply.rs` writes files; tests use no real-network URLs;
`scripts/privacy-check.sh --all`.

### Review changes
- Round 1: S6's slow probe path now answers 403, so only the probe timeout keeps the link from being BLOCKED.
