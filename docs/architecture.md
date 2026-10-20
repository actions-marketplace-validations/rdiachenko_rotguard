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

- **Linked websites**: a timeout, TLS error, 401, 403, 429 or 5xx makes a link `UNVERIFIABLE`,
  not dead. Only a 404, 410 or a missing host on two tries makes it `DEAD`.
- **Wayback Machine**: when it fails or has no capture, the finding says `manual fix: no
  archived copy` or `archive unreachable`, and nothing is written.

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
  `Verdict` (`Ok`, `Permanent`, `Temporary`, `Dead`, `Suspect`, `Unverifiable`). `check_all`
  checks every distinct URL with retries, re-checks a first `Dead`, runs the soft-404 checks and
  stops at the deadline.
- `archive.rs`: Wayback CDX lookups, the latest status-200 capture on or before the page date,
  else the earliest after it, through one shared rate gate.
- `apply.rs`: writes proposed fixes for permanent redirects and dead links. It writes only
  markdown link destinations and TOML front-matter strings, re-reads each file first, and
  replaces it atomically.
- `report.rs`: renders findings and the summary line as text, JSON or HTML.
- `progress.rs`: progress on stderr, one rewritten line on a terminal, plain lines when piped.
- `lib.rs`: `run(Args) -> ExitCode` wires it together; `main.rs` only parses arguments.

## Main flows

**Check** (every run):
1. Load the config; read every file matching `content`, sorted by path; extract links.
2. Drop internal links (host of `site_url`) and `ignore` matches; strip fragments; dedupe.
3. `check_all` the distinct URLs until `deadline_s`. A URL still running then is
   `UNVERIFIABLE` (`deadline`).
4. Look up a snapshot for each dead or suspect link at its page's date (the run date when the
   page has none).
5. Build one finding per occurrence: a permanent redirect proposes its target, dead and suspect
   propose the snapshot, temporary and unverifiable propose nothing.
6. Print the report; exit 1 if a dead, permanent or suspect link is left (with `--strict`, also
   temporary and unverifiable), else 0.

**Soft-404 detection**: a 2xx answer is suspect when the page redirected a non-root URL to the
site root, when its title matches a not-found phrase, or when a probe URL with a random last
segment under the same parent path returns the same page. Probes are shared per origin and
parent path. A host with `soft404 = false` skips these checks.

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
- One `--format` flag picks text, JSON or HTML; JSON keys are stable and always present
  (`docs/design/report-format.md`, D1, D3).
- `robots.txt` is not consulted (`docs/design/link-health.md`, D9).

## Limits

- Markdown sites only; no HTML crawling and no internal-link checks.
- No state between runs: no history, caching or scheduled runs.
- A site that blocks rotguard (403 or 429 to every request) shows up as unverifiable links.
