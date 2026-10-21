# Architecture

rotguard is a command-line tool that checks the outbound links of a markdown site for rot: dead
links, redirects and soft-404s. It reports each broken link with a proposed fix, and with
`--apply` writes the safe fixes back into the files.

## Context

```text
                 ┌──────────────┐
                 │ Site owner   │
                 └──────┬───────┘
  rotguard [--apply]    │ ▲ report on stdout, progress on stderr,
  [--strict] [--format] │ │ exit code 0 / 1 / 2
                 ┌──────▼─┴─────────────┐  reads rotguard.toml and   ┌──────────────────────┐
                 │ rotguard             ├───────────────────────────►│ Site directory       │
                 │ [Rust, one binary]   │  content files; --apply    │ rotguard.toml,       │
                 │                      │  rewrites them             │ content/**/*.md      │
                 └──────┬──────────┬────┘                            └──────────────────────┘
       HEAD/GET each    │          │ snapshot lookups (dead and suspect links only)
       external URL     ▼          ▼
            ┌──────────────────┐ ┌──────────────────────────┐
            │ Linked websites  │ │ Wayback Machine CDX API  │
            └──────────────────┘ └──────────────────────────┘
```

- **Linked websites**: a timeout, TLS error, failed DNS lookup, 429 or 5xx makes a link
  `UNVERIFIABLE`, not dead. A 401, 403 or 999 is `BLOCKED` when a made-up URL beside it gets the
  same status, else `UNVERIFIABLE`. Only a 404, 410 or a host DNS says doesn't exist, on two
  tries, makes a link `DEAD`.
- **Wayback Machine**: when it fails or has no capture, the finding says `manual fix: no
  archived copy` or `archive unreachable`, and nothing is written.
- **GitHub Actions**: the site owner can also run the binary from `action.yml` on a schedule. The
  site's workflow (`docs/github-actions.md`) opens a pull request with the fixes.

## Components

One crate, `src/`:

- `config.rs`: loads and validates `rotguard.toml` into `Config`. Unknown keys are errors. Per-host
  settings (`[hosts."<host>"]`, today only `soft404`) live in `HostConfig`.
- `extract.rs`: `extract(path, text, cfg) -> Page`, the links in one file with their line, the
  syntax they were written in (`Ctx`), whether they are `verbatim` (written so `--apply` can
  replace them) and the page date from front matter. Covers markdown links, images, reference
  definitions, autolinks, raw-HTML `href`/`src`, bare URLs, and TOML and YAML front matter.
- `check.rs`: the network. `Fetcher` sends requests politely: one in flight per host, spaced
  starts, a bound on hosts at once, redirects followed by hand. `classify` turns a fetch into a
  `Verdict` (`Ok`, `Permanent`, `Temporary`, `Dead`, `Suspect`, `Unverifiable`, `Blocked`).
  `check_all` checks every distinct URL with retries, re-checks a first `Dead`, runs the soft-404
  and bot-wall probes and stops at the deadline.
- `archive.rs`: Wayback CDX lookups, the latest status-200 capture on or before the page date,
  else the earliest after it, through one shared rate gate.
- `apply.rs`: writes proposed fixes for permanent redirects and dead links. It writes only
  markdown link destinations and TOML front-matter strings, re-reads each file first, and
  replaces it atomically.
- `report.rs`: renders findings and the summary line as text, JSON, HTML or markdown.
- `progress.rs`: progress on stderr, one rewritten line on a terminal, plain lines when piped.
- `lib.rs`: `run(Args) -> ExitCode` wires it together; `main.rs` only parses arguments.

Outside `src/`, `action.yml` is a composite GitHub Action. It builds rotguard on the runner, runs
`--apply --format markdown`, writes the report to the job summary and outputs the exit code and
the path of the report. The site's workflow, shown in `docs/github-actions.md`, uses those
outputs to open the pull request.

## Main flows

**Check** (every run):
1. Load the config; read every file matching `content`, sorted by path; extract links.
2. Drop internal links (host of `site_url`) and `ignore` matches; strip fragments; dedupe.
3. `check_all` the distinct URLs until `deadline_s`. A URL still running then is
   `UNVERIFIABLE` (`deadline`).
4. Look up a snapshot for each dead or suspect link at its page's date (the run date when the
   page has none).
5. Build one finding per occurrence: a permanent redirect proposes its target, dead and suspect
   propose the snapshot, temporary, unverifiable and blocked propose nothing.
6. Print the report; exit 1 if a dead, permanent or suspect link is left (with `--strict`, also
   temporary, unverifiable and blocked), else 0. Blocked links are listed only with `--strict`;
   the summary always counts them.

**Soft-404 detection**: a 2xx answer is suspect when the page redirected a non-root URL to the
site root, when its title matches a not-found phrase, or when a probe URL with a random last
segment under the same parent path returns the same page. Probes are shared per origin and
parent path. A host with `soft404 = false` skips these checks.

The same probe decides bot walls: a link whose final status is 401, 403 or 999 is `Blocked`
when the probe gets that status too, and `Unverifiable` when it answers differently or fails.
This runs whatever the host's `soft404` setting, and shares the probe cache.

**Apply** (`--apply`, after the check): per file, confirm each finding's span still holds its
original URL, then rewrite and rename into place. A fix that can't be written safely becomes
`manual fix: <reason>`; a file that changed during the run is left untouched (`changed during
run`). An I/O error leaves that file as it was and exits 2.

## Data

rotguard stores nothing between runs. It reads `rotguard.toml` and the content files, and with
`--apply` rewrites content files in place. Its only output is stdout, stderr and the exit code.

## Decisions

- Report by default; `--apply` writes only fixes that can't be wrong: permanent-redirect targets
  and snapshots for dead links, never suspects or unverifiable links
  (`docs/design/link-health.md`, D1, D6).
- `--apply` rewrites only markdown link destinations and TOML front-matter strings; YAML, raw
  HTML and bare URLs are reported as `manual fix: not verbatim` (`docs/design/simplify.md`, D1, D3).
- A dead link's fix is the snapshot closest to the page's date (`docs/design/link-health.md`, D2).
- Soft-404s are detected by probing, not by a list of known sites (`docs/design/link-health.md`, D4).
- Progress goes to stderr, so stdout stays a clean report for scripts
  (`docs/design/progress-and-config.md`, D1).
- One `--format` flag picks text, JSON, HTML or markdown; JSON keys are stable and always present
  (`docs/design/report-format.md`, D1, D3).
- `robots.txt` is not consulted (`docs/design/link-health.md`, D9).
- A 401, 403 or 999 is a bot wall only when a same-folder probe gets the same status, and blocked
  links are listed only with `--strict` (`docs/design/bot-walls.md`, D1, D2).
- The weekly run lives in CI, not in the binary: a composite action builds rotguard from its own
  source with no secret, and only exit 0 or 1 opens a PR (`docs/design/weekly-fix-pr.md`, D1, D6).
- The PR body is the markdown report, rendered by rotguard; the log and the job summary show the
  same report (`docs/design/readable-pr-body.md`, D1, D4).
- A DNS failure is dead only when the answer says the name doesn't exist; any other lookup
  failure is unverifiable, so it never leads to a write (`docs/design/dns-hiccup.md`, D1).

## Limits

- Markdown sites only; no HTML crawling and no internal-link checks.
- No state between runs: no history, caching or scheduled runs.
- A site that answers 429 to every request shows up as unverifiable links.
