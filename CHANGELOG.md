# Changelog

All notable changes to rotguard are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

First public release.

- Checks the outbound links of a markdown site's sources: markdown links, reference definitions,
  autolinks, raw HTML `href`/`src`, bare URLs and TOML and YAML front matter.
- Classifies each link as permanent or temporary redirect, dead, suspect (soft-404),
  unverifiable or blocked (bot wall).
- Proposes a fix for permanent redirects and dead links, the latter from the Wayback Machine
  snapshot closest to the page's date. `--apply` writes the safe fixes back into the files.
- Reports as text, JSON, HTML or markdown; `--strict` and `--quiet` flags.
- A composite GitHub Action that runs `--apply` and outputs a markdown report for a pull request.
