# Changelog

All notable changes to rotguard are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0] - 2026-11-08

- Fewer false soft-404s on sites unlike a personal blog:
  - A link is no longer suspect when the made-up probe URL redirects to the link itself, as on
    sites that find an article by its ID and ignore the rest of the URL.
  - A shared `<title>` no longer makes a link suspect when the two pages' text differs.
- A link that looks like a soft-404 is now `UNVERIFIABLE` instead of suspect when the host's root
  gets the same page as the probe (`probe: every URL gets the same page`) or the page asks for
  JavaScript (`probe: page needs JavaScript`). Detection may GET a host's root once.
- Snapshot lookups run for dead links first, then suspects, so a deadline cuts off suspects first.
- The action takes a `path` input (default `.`) for a site in a subfolder.

## [0.1.1] - 2026-11-08

- The action is named "rotguard link checker & fixer" so it can be listed on the GitHub
  Marketplace, where the name "rotguard" is taken by a user account. `uses: rdiachenko/rotguard@v0`
  is unchanged.

## [0.1.0] - 2026-11-08

First public release.

- Checks the outbound links of a markdown site's sources: markdown links, reference definitions,
  autolinks, raw HTML `href`/`src`, bare URLs and TOML and YAML front matter.
- Classifies each link as permanent or temporary redirect, dead, suspect (soft-404),
  unverifiable or blocked (bot wall).
- Proposes a fix for permanent redirects and dead links, the latter from the Wayback Machine
  snapshot closest to the page's date. `--apply` writes the safe fixes back into the files.
- Reports as text, JSON, HTML or markdown; `--strict` and `--quiet` flags.
- A composite GitHub Action that runs `--apply` and outputs a markdown report for a pull request.
- Every release publishes static Linux binaries for x86_64 and arm64 on the GitHub releases page.
- The action pinned to `@v0` downloads the release binary instead of building rotguard; other
  refs build from source as before.
- Without system CA certificates rotguard prints a message and exits 2 instead of panicking.
