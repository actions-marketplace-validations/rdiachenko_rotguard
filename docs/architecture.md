# Architecture

rotguard is a command-line tool for people who keep a markdown site. It checks the site's
outbound links for rot (dead links, redirects and soft-404s), reports each broken link with a
proposed fix, and with `--apply` writes the safe fixes back into the files. Start reading at
`run` and `check_pages` in `src/lib.rs`, then `Verdict` and `check_all` in `src/check.rs`.

## Context

```text
Context: rotguard

                  ┌──────────────┐
                  │ Site owner   │
                  └──────┬───────┘
  rotguard [site_dir]    │ ▲ report on stdout, progress on stderr,
  [--apply] [--strict]   │ │ exit code 0 / 1 / 2
  [--quiet] [--format]   │ │
                  ┌──────▼─┴─────────────┐  reads rotguard.toml and   ┌──────────────────────┐
                  │ rotguard             ├───────────────────────────►│ Site directory       │
                  │ [Rust, one binary]   │  content; --apply replaces │ rotguard.toml,       │
                  │                      │  files via a temp file     │ content/**/*.md      │
                  └──────┬──────────┬────┘                            └──────────────────────┘
       GET each external │          │ snapshot lookups (dead and suspect links only)
       URL (HTTPS)       ▼          ▼
            ┌──────────────────┐ ┌──────────────────────────┐
            │ Linked websites  │ │ Wayback Machine CDX API  │
            └──────────────────┘ └──────────────────────────┘
```

- **Linked websites**: a timeout, TLS error, failed DNS lookup, 429 or 5xx makes a link
  `UNVERIFIABLE`, never dead, after 2 retries (a 429 once, honouring `Retry-After` up to 30 s).
  A 401, 403 or 999 is `BLOCKED` when a made-up URL beside it gets the same status, else
  `UNVERIFIABLE`. Only a 404, 410 or a DNS answer that the name doesn't exist, on 2 tries,
  makes a link `DEAD`.
- **Wayback Machine**: 3 attempts per query. When it fails, has no capture or runs past the
  deadline, the finding says `manual fix: archive unreachable`, `no archived copy` or
  `deadline`, and nothing is written.
- **GitHub Actions**: `action.yml` runs the binary on the site's runner, usually weekly, and the
  site's workflow (`docs/github-actions.md`) opens a pull request with the fixes. A failed
  build or a cache miss never changes what rotguard reports.

## Components

```text
Components: one crate, src/

  main.rs ──Args──► lib.rs: run, check_pages, exit_code
                     │ 1 load      │ 2 extract       │ 3 check_all      │ 4 lookup
                     ▼             ▼                 ▼                  ▼
                 config.rs     extract.rs        check.rs           archive.rs
                 Config        Page, Link, Ctx   Fetcher, Verdict   CDX gate
                     │ 5 findings with Fix::Proposed
                     ▼
                 apply.rs ──temp file + rename──► content files   (the only writer)
                     │ 6 findings, Summary
                     ▼
                 report.rs ──text/JSON/HTML/markdown──► stdout
                 progress.rs ──progress lines──► stderr
```

`lib.rs` owns the run. `run(Args) -> ExitCode` loads the config, reads the content files,
calls `check_pages` and decides the exit code in `exit_code`. It also owns the `Finding`, `Fix`
and `ManualReason` types every other module passes around. `main.rs` only parses arguments.

`check.rs` owns the network and the verdicts. `Fetcher` sends GET requests politely: 1 in
flight per host, spaced starts, at most 8 hosts at once, redirects followed by hand up to 10.
`classify` turns a fetch into a `Verdict` (`Ok`, `Permanent`, `Temporary`, `Dead`, `Suspect`,
`Unverifiable`, `Blocked`). `check_all` checks every distinct URL with retries, re-checks a first
`Dead`, runs the soft-404 and bot-wall probes and stops at the deadline. It never writes.

`apply.rs` is the only module that writes files, a rule a fitness check in `.whittle/gates.toml`
enforces. `apply` writes fixes only for `Permanent` and `Dead` findings with a proposed fix, and
only into markdown link destinations and quoted TOML front-matter strings. It never applies a
fix for a suspect, temporary, unverifiable or blocked link.

`extract.rs` owns link discovery. `extract(path, text, cfg) -> Page` returns each link with its
line, its byte span, the syntax it was written in (`Ctx`) and whether it is `verbatim`
(written so `--apply` can replace the span), plus the page date from front matter. It covers
markdown links, reference definitions, autolinks, raw-HTML `href`/`src`, bare URLs, and TOML
and YAML front matter.

`config.rs` loads `rotguard.toml` into `Config` and `HostConfig` and rejects unknown keys.
`archive.rs` finds the Wayback capture closest to a page date, with 1 request in flight at a
time. `report.rs` renders findings and the summary in 4 formats and has no say in the exit
code. `progress.rs` writes progress to stderr only, so stdout stays a clean report.

Outside `src/`, `action.yml` is a composite GitHub Action with 5 steps:
1. Pick a cache key from the OS, the stable `rustc` version and `Cargo.lock`.
2. Restore the build cache.
3. Build rotguard.
4. Run `--apply --format markdown`, always exiting 0 with the real code in the `exit-code`
   output.
5. Save the cache on a miss.

## Main flows

**Check** (every run):
1. Load the config; read every file matching `content`, sorted by path; extract links. A config,
   glob or read error exits 2.
2. Drop internal links (host of `site_url`) and `ignore` matches; strip fragments; dedupe.
3. `check_all` the distinct URLs until `deadline_s`. A URL still running then is
   `UNVERIFIABLE` (`deadline`).
4. Look up a snapshot for each dead or suspect link at its page's date (the run date when the
   page has none).
5. Build 1 finding per occurrence: a permanent redirect proposes its target, dead and suspect
   propose the snapshot, temporary, unverifiable and blocked propose nothing.
6. With `--apply`, apply the fixes. Print the report; exit 1 if a dead, permanent or suspect
   link is left unfixed (with `--strict`, also temporary, unverifiable and blocked), else 0.
   Blocked links are listed only with `--strict`; the summary always counts them.

**Soft-404 and bot-wall probes**: a 2xx answer is suspect when a non-root link ends at the site
root or its permanent fix would point there (a middle hop through `/` doesn't count), when its
title matches a not-found phrase, or when a probe URL with a random last segment under the same
parent path returns the same page. A link ending in `/` is probed as a sibling folder
(`/a/b/` probes `/a/<token>/`); other paths probe `/a/<token>`. Probes are shared per origin,
parent path and trailing slash. The same probe decides bot walls: a final 401, 403 or 999 is
`Blocked` when the probe gets that status too. A host with `soft404 = false` skips the soft-404
checks but keeps the bot-wall probe.

**Apply** (`--apply`, after the check): per file, re-read it and confirm each finding's span
still holds its original URL, then write `<file>.rotguard.tmp` and rename it over the file. A
fix that can't be written safely becomes `manual fix: <reason>`; a file that changed during the
run is left untouched (`changed during run`). An I/O error leaves that file as it was, reports
its fixes as `manual fix: write failed` and exits 2.

**Weekly fix PR** (`docs/github-actions.md`): the site's workflow runs the action. On exit 0
or 1 it opens or updates a PR whose body is the markdown report; on 1 or anything higher the job
fails afterwards. On exit 2 or higher no PR opens, because `--apply` may have written some files
and not others.

## Data

The binary keeps nothing between runs: no history and no link cache. It reads `rotguard.toml`
and the content files, and with `--apply` replaces content files through a temp file in the
same directory. Its output is stdout, stderr and the exit code.

The action writes the report to `$RUNNER_TEMP/rotguard-report.md` and the job summary, and keeps
the compiled build in the site repo's Actions cache under
`rotguard-<os>-<rustc>-<Cargo.lock hash>`.

## Hard parts

- **Writing the user's files.** A stale span would corrupt text the user edited mid-run, and a
  crash mid-write would leave half a file. `apply` re-reads each file and checks every span
  still holds its URL, applies edits last span first, refuses overlapping spans, creates the
  temp file with `create_new` (so a planted symlink is never followed), syncs it and renames it.
  The unit tests in `src/apply.rs` cover each case.
- **Spans and `verbatim` in extract.** A wrong span or `verbatim` flag makes apply write into
  the wrong bytes. TOML strings, YAML, CRLF, a byte-order mark and escaped or entity-encoded
  destinations are where it goes wrong. YAML, HTML and escaped links are never verbatim; tests
  in `src/extract.rs` and `tests/acceptance/extract.rs` pin each form.
- **Politeness and the deadline.** `Fetcher` takes the host gate before the global permit, so
  one slow host can't starve the others, and every wait is bounded by the deadline, after which
  `run` drops the runtime without waiting for hung requests. Tests in `src/check.rs` and
  `src/archive.rs` check spacing, 1 request per host and hanging servers.
- **Soft-404 judgement.** It decides between `Ok`, `Suspect` and `Blocked` from a shared probe
  cache, so a change to a probe key or to the root-redirect rule shifts verdicts across links.
  `tests/acceptance/soft404.rs`, `blocked.rs`, `directory_probe.rs` and `root_redirect.rs`
  hold the agreed cases.

## Decisions

- Report by default; `--apply` writes only fixes that can't be wrong: permanent-redirect targets
  and snapshots for dead links, never suspects or unverifiable links
  (`docs/design/link-health.md`, D1, D6).
- `--apply` rewrites only markdown link destinations and TOML front-matter strings; YAML, raw
  HTML and bare URLs are reported as `manual fix: not verbatim` (`docs/design/simplify.md`, D1, D3).
- A dead link's fix is the snapshot closest to the page's date (`docs/design/link-health.md`, D2).
- Soft-404s are detected by probing, not by a list of known sites
  (`docs/design/link-health.md`, D4).
- `robots.txt` is not consulted (`docs/design/link-health.md`, D9).
- Progress goes to stderr, so stdout stays a clean report for scripts
  (`docs/design/progress-and-config.md`, D1).
- One `--format` flag picks text, JSON, HTML or markdown; JSON keys are stable and always present
  (`docs/design/report-format.md`, D1, D3).
- A 401, 403 or 999 is a bot wall only when a same-folder probe gets the same status, and blocked
  links are listed only with `--strict` (`docs/design/bot-walls.md`, D1, D2).
- A DNS failure is dead only when the answer says the name doesn't exist; any other lookup
  failure is unverifiable, so it never leads to a write (`docs/design/dns-hiccup.md`, D1).
- A fix `--apply` couldn't write is a hand fix, marked in apply so every report format says so
  (`docs/design/apply-failed-fix.md`, D1).
- A link ending in `/` is probed with a sibling that ends in `/` too, so the probe takes the same
  route through the site's server as the link (`docs/design/directory-probe.md`, D1).
- A redirect to the site root counts only where the link ends up or where its fix would point,
  so a chain that goes on to a real page keeps its own soft-404 checks
  (`docs/design/root-redirect.md`, D1).
- The weekly run lives in CI, not in the binary: a composite action builds rotguard from its own
  source with no secret, and only exit 0 or 1 opens a PR (`docs/design/weekly-fix-pr.md`, D1, D6).
- The action restores and saves its build cache in separate steps, the save inside the action, so
  a week whose job fails on a hand fix still saves (`docs/design/build-cache.md`, D3).
- The PR body is the markdown report, rendered by rotguard; the log and the job summary show the
  same report (`docs/design/readable-pr-body.md`, D1, D4).

## Limits

- Markdown sources only: no crawling of built HTML and no internal-link or anchor checks.
- No memory between runs, so a host that is down on the day of a run shows up that week.
- A site that answers 429 to every request shows up as unverifiable links.
- `--apply` never edits YAML front matter, raw HTML or bare URLs; those stay hand fixes.
