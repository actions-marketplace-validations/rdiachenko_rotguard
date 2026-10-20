---
status: approved
---
# Progress output and a config reference

## Problem
A run on rdiachenko.com checks about 90 distinct URLs on 37 hosts. github.com alone has 33 of
them, and requests to one host start at least 1 s apart, so a healthy run takes 40–60 s. Each
dead link adds a 5 s recheck and 2–6 archive requests 4 s apart. Nothing is printed until the
final report, so the run looks hung for a minute or two. Separately, the README names 7 of the 13
config keys without saying what they do. Its `[hosts."goodreads.com"]` example is wrong, because
host keys match exactly and the site needs `www.goodreads.com`. The user wants to see progress,
and wants every config field explained.

## Scope
In:
- Progress on stderr, ~95 lines: a live counter in a terminal and plain phase lines otherwise.
  The user runs rotguard by hand in a terminal; plain lines suit CI and pipes.
- `docs/config.md`, a reference for every key (docs, no code). The README's Configure section
  links to it, and its example is fixed. The user asked for every field to be clear.

Out:
- A `--quiet` or `--progress` flag (parked as an idea): plain mode prints at most 4 lines.
- Per-URL lines, ETAs, spinners, colours, or a progress crate.
- Progress for config load, the file walk, extraction and `--apply`: each takes milliseconds.
- Any change to the report on stdout, the exit codes or the config keys themselves.

Size: ~95 non-test lines in total (`src/progress.rs` ~65, plumbing in check.rs and lib.rs ~30).

## Requirements
- R1: THE SYSTEM SHALL write progress only to stderr. Stdout keeps exactly the report it prints
  today.
- R2: WHILE stderr is not a terminal THE SYSTEM SHALL print the line `checking <N> <link|links>
  on <H> <host|hosts>` when the check phase starts, and `checked <N> <link|links> in <T>s` when
  it ends. N is the number of distinct URLs checked, H the number of distinct hosts among them,
  T the phase's elapsed whole seconds rounded down. The singular form is used when the count is 1.
- R3: WHILE stderr is not a terminal, WHEN at least one archive lookup is needed THE SYSTEM
  SHALL print `looking up <N> <snapshot|snapshots>` when the lookups start, and `looked up <N>
  <snapshot|snapshots> in <T>s` when they end. N is the number of distinct (URL, page date)
  lookups. It can exceed the number of dead or suspect URLs when one URL is linked from pages
  with different dates. With no lookup, no archive line is printed.
- R4: WHILE stderr is a terminal THE SYSTEM SHALL show one line rewritten in place. During
  checking it reads `checking links: <D>/<N> done`, and during lookups `looking up snapshots:
  <D>/<N> done`. It is redrawn when the phase starts (D = 0) and each time a URL or lookup
  finishes. When the phase ends, the line is cleared and replaced by the R2 or R3 end line,
  which is followed by a newline.
- R5: IF the run exits before checking starts (a missing or invalid `rotguard.toml`, a bad
  content glob) THEN THE SYSTEM SHALL print no progress line.
- R6: THE SYSTEM SHALL ship `docs/config.md`, which documents every key `rotguard.toml` accepts
  (the 12 top-level keys and `[hosts."<name>"]` with `soft404`). Each entry gives the type, the
  default, the valid values and the exact effect, as the code behaves. The page ends with one
  TOML block that lists every top-level key at its default value.
- R7: THE SYSTEM SHALL link `docs/config.md` from the README's Configure section. The README's
  example config SHALL load without error and use `www.goodreads.com` as its host key.

## Acceptance scenarios
- S1 [R1, R2]: Given `post.md` with links `M/a` and `M/b`, both answering 200 (one mock host),
  when rotguard runs with stderr piped, then stderr has the line `checking 2 links on 1 host`
  followed later by a line matching `^checked 2 links in [0-9]+s$`. Stdout contains no line
  starting with `checking ` or `checked 2 links`, and its last line is the summary line.
- S2 [R3]: Given `post.md` with `M/gone` answering 404 twice and a CDX capture for it, when
  rotguard runs, then stderr has the lines `looking up 1 snapshot` and one matching
  `^looked up 1 snapshot in [0-9]+s$`, in that order and after the `checked 1 link in …` line.
- S3 [R3]: Given `post.md` whose only link `M/a` answers 200, when rotguard runs, then stderr
  contains no line starting with `looking up` or `looked up`.
- S4 [R4]: Given a `Progress` in terminal mode writing into a buffer, when the check phase starts
  with 2 URLs on 1 host, one URL finishes, the second finishes and the phase ends, then the
  buffer reads `\rchecking links: 0/2 done`, `\rchecking links: 1/2 done`,
  `\rchecking links: 2/2 done`, and finally `\r\x1b[2K` followed by a line matching
  `checked 2 links in [0-9]+s\n`, with nothing in between.
- S5 [R5]: Given a site directory with no `rotguard.toml`, when rotguard runs, then it exits 2
  and stderr contains no line starting with `checking`.
- S6 [R6]: Given the last TOML block of `docs/config.md` saved as `rotguard.toml` on a site with
  one link-free page, when rotguard runs, then it exits 0. That block's top-level keys are
  exactly `site_url`, `content`, `date_key`, `user_agent`, `ignore`, `not_found_phrases`,
  `archive_url`, `host_interval_ms`, `archive_interval_ms`, `recheck_delay_ms`, `timeout_ms`
  and `deadline_s`. The page has a heading containing each of them and one containing `hosts`.
- S7 [R7]: Given the TOML block in README.md's Configure section saved as `rotguard.toml` on a
  site with one link-free page, when rotguard runs, then it exits 0. The block contains
  `[hosts."www.goodreads.com"]`, and the Configure section links to `docs/config.md`.

## Design

```text
            ┌──────────────── rotguard ────────────────┐
  site ────►│ lib::run ──urls──► check::check_all       │──report──► stdout (unchanged)
  files     │    │                 │ on_done(D)         │
            │    │       ┌─────────▼────────┐           │
            │    └──────►│ progress::Progress│──lines───┼──────────► stderr
            │  lookups   └─────────▲────────┘           │
            │    └───► look_up_snapshots ──on_done(D)   │
            └───────────────────────────────────────────┘
```

New module `src/progress.rs`, which owns everything progress prints:

```rust
pub enum Phase { Check { hosts: usize }, Archive }

pub struct Progress<W: Write> { out: W, live: bool, phase: Option<(Phase, usize, Instant)> }

impl Progress<io::Stderr> {
    /// Live when stderr is a terminal (`std::io::IsTerminal`).
    pub fn stderr() -> Self;
}
impl<W: Write> Progress<W> {
    pub fn new(out: W, live: bool) -> Self;
    pub fn start(&mut self, phase: Phase, total: usize); // plain: start line; live: "D = 0" line
    pub fn tick(&mut self, done: usize);                 // live: redraw; plain: nothing
    pub fn finish(&mut self);                            // live: "\r\x1b[2K" + end line; plain: end line
}
```

- Write errors on stderr are ignored: progress must never fail a run.
- Wording and plural forms live in `progress.rs` only. Seconds are `elapsed.as_secs()`.
- `check::check_all(urls, cfg, deadline, on_done: impl FnMut(usize))` calls `on_done(done)` in
  its `join_next` loop after each verdict. A URL cut off by the deadline counts as done.
  `look_up_snapshots` in lib.rs gains the same parameter.
- `src/lib.rs` declares `pub mod progress;`. S4 is the one acceptance test that drives the
  library instead of the binary: it builds `rotguard::progress::Progress::new(&mut buf, true)`
  over a `Vec<u8>` and reads `buf` once the `Progress` is dropped. A terminal can't be faked
  through the binary's piped stderr.
- `lib::check_pages` counts the distinct hosts of the `distinct` URLs, then calls
  `start(Phase::Check { hosts }, distinct.len())` before `check_all` and `finish()` after it.
  When `lookups` is not empty, it calls `start(Phase::Archive, lookups.len())` and `finish()`
  around `look_up_snapshots`.
  `Progress::stderr()` is created after config load and extraction succeed, so R5 holds by
  construction.
- Flow: config → extract → `checking …` → per-URL ticks → `checked …` → (lookups:
  `looking up …` → ticks → `looked up …`) → apply → report on stdout.
- `docs/config.md`: one `###` heading per key, in config order. Each states type, default,
  valid values and effect, taken from src/config.rs and the code that uses it:
  - `site_url`: exact host match, with no `www.` folding and no subdomains.
  - `content`: globs relative to the site directory, where `*` stays within one path segment
    and `**` crosses them.
  - `date_key`: a TOML datetime, or a string whose first 10 characters are `YYYY-MM-DD`; only
    used to pick a snapshot.
  - `user_agent`: sent on checks, probes and archive queries.
  - `ignore`: an unanchored regex search on the URL as written, fragment included, applied after
    the internal-host check.
  - `not_found_phrases`: matched against the page title.
  - `archive_url`: the base for CDX queries and for the proposed snapshot URLs.
  - `host_interval_ms`: start-to-start spacing per host, counting every request.
  - `archive_interval_ms`: one global spacing between archive requests.
  - `recheck_delay_ms`: the wait before re-fetching a dead URL.
  - `timeout_ms`: a limit per request, not per redirect chain.
  - `deadline_s`: covers checking and archive lookups together.
  - `[hosts."<name>"]`: an exact host match after lowercasing.

  Each entry also names the error when a value is invalid. The builder confirms each statement
  against the code (for example, the case rule of `not_found_phrases`) before writing it.

## Decisions
### D1: Progress goes to stderr
Options: stdout before the report; stderr. Chosen: stderr. Why: the report stays a clean,
pipeable stdout, and the acceptance harness reads the summary from stdout's last line. Cost if
wrong: low; moving the stream is one constructor.

### D2: Detect the terminal with `std::io::IsTerminal`, with no flag
Options: a progress crate; a `--progress`/`--quiet` flag; std detection. Chosen: std detection
on stderr. Why: no new dependency, and plain mode prints at most 4 lines, which is fine in CI.
Cost if wrong: low; a flag can be added later (parked as an idea).

### D3: Redraw on every completion, with no throttle
Options: a timer; a redraw per completion. Chosen: a redraw per completion. Why: about 100
redraws per run on the user's site is negligible, and it needs no timer task. Cost if wrong: low.

### D4: The config reference is `docs/config.md`, linked from the README
Chosen by the user. Why: the README stays lean; the reference has room for matching rules and
errors. Cost if wrong: low; the content can move.

## Risk paths
None new. `.whittle/gates.toml` keeps `src/apply.rs`, `Cargo.toml` and `Cargo.lock`. This design
touches none of them.

## Fitness rules
Unchanged: only `src/apply.rs` writes files; tests use no real-network URLs;
`scripts/privacy-check.sh --all`.

## Review changes
- Round 1: S4 is stated as a library-level acceptance test of `rotguard::progress`, which `src/lib.rs` declares `pub`.
- Round 1: R3's N is defined as the number of distinct (URL, page date) lookups, and the Design passes `lookups.len()`.

## Open for you
none
