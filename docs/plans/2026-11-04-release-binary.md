# Plan: Release binaries and a pinned action
Design: docs/design/release-binary.md
Risk paths: `scripts/release-verify.sh` (T3), `scripts/release-publish.sh` (T4), `.github/workflows/release.yml` (T5), `action.yml` (T6) — the release workflow, its 2 scripts and the action's fetch step, as the design's `Touches:` approves.
Diff estimate: 250

The design's 3 unverified facts (the action folder name for tag refs, `v0` staying
force-pushable with immutable releases on, `if:` on composite steps) need a published tag on
GitHub, which this branch can't create. Each has a fallback that is slower but never wrong, so
no probe task runs here: T7 writes their checks into the `CLAUDE.md` first-release steps.

S14 is Linux-only (`#[cfg(target_os = "linux")]`). On macOS the build proves it red and green in
Docker as a non-root user:
`docker run --rm --user 1000:1000 -e HOME=/tmp -e CARGO_HOME=/tmp/cargo -e CARGO_TARGET_DIR=/tmp/t -v "$PWD":/src:ro -w /src rust:1 cargo test --locked --test acceptance ca_certs -- --include-ignored`.

### T1: Acceptance tests
- Files: tests/acceptance/release.rs, tests/acceptance/fetch.rs, tests/acceptance/ca_certs.rs, tests/acceptance/main.rs
- Interfaces: Consumes `tests/acceptance/support.rs` (`World`, `w.site.write`, `w.url`) to set
  up S14's site, and repo files read from `env!("CARGO_MANIFEST_DIR")`:
  `scripts/release-verify.sh`, `scripts/release-publish.sh`, `.github/workflows/release.yml`,
  `action.yml`. Script and step tests follow `tests/acceptance/build_cache.rs`: parse YAML with
  `serde_norway`, write a step's `run:` text to a temp `step.sh`, run
  `bash --noprofile --norc -eo pipefail` with stub executables first on `PATH`.
  - `release.rs`: S1, S2, S3 run `scripts/release-verify.sh <notes>` in a temp repo dir holding
    a `Cargo.toml` and `CHANGELOG.md`, with `GITHUB_REF_NAME` set. S5, S6, S17 run
    `scripts/release-publish.sh v0.1.0 <dist> <notes>` with `GITHUB_SHA` set to 40 hex digits
    and stub `gh` and `git` that append `gh <args>` / `git <args>` lines to a log file named by
    an env var; the `gh` stub fails on a subcommand named by another env var. S5 compares
    `SHA256SUMS` with sums computed by running `sha256sum` (or `shasum -a 256` where absent) on
    the fixtures. S4 parses `.github/workflows/release.yml`.
  - `fetch.rs`: S7, S8, S10, S11, S12, S13, S16, S18 run the `id: fetch` step's script with
    `GITHUB_ACTION_PATH` = `<tmp>/_actions/o/r/<ref>` (holding a `Cargo.toml` at 0.1.0),
    `RUNNER_OS`, `RUNNER_ARCH`, `RUNNER_TEMP`, `GITHUB_OUTPUT`, and a stub `curl` that writes a
    fixture to the argument after `--output`, logs its last argument (the URL) to a file named
    by an env var, and exits with a code from another env var. URL assertions match the suffix
    `github.com/rdiachenko/rotguard/releases/download/...`, never an `https://` literal (fitness
    rule 2). S9 parses `action.yml`.
  - `ca_certs.rs`: starts with `#![cfg(target_os = "linux")]`, so the whole module, imports
    included, compiles only on Linux and clippy stays clean on macOS. S14 sets up a site with 1
    link to `w.url(...)`, then starts its own
    `Command::new(env!("CARGO_BIN_EXE_rotguard")).arg(w.site.path())` with `.env("SSL_CERT_FILE",
    <empty dir>/none.pem).env("SSL_CERT_DIR", <empty dir>)`, and checks exit code 2, the
    `rotguard: ` prefix on stderr and no `panicked`. Never `std::env::set_var`: it would leak
    into every acceptance test running at the same time. `support.rs` stays untouched.
  - `main.rs` is locked: open it with `whittle-gates amend` and add `mod ca_certs; mod fetch;
    mod release;` only.
- Test: one test per scenario, named `<module>_s<n>_<what>`, each marked
  `#[ignore = "pending: S<n>"]` with its own S-ID. S15 is not here: it is the existing
  `action_s5` test, changed in T7.
- Covers: S1, S2, S3, S4, S5, S6, S7, S8, S9, S10, S11, S12, S13, S14, S16, S17, S18
- Commit: Add acceptance tests for release binaries

### T2: Exit cleanly without CA certificates
- Files: src/check.rs, src/archive.rs, src/lib.rs
- Interfaces: The 2 clients differ today: the checker's turns redirects off
  (`Policy::none()`), the archive's follows them. Keep both settings exactly.
  - Produces `check::http_client(cfg: &Config) -> Result<reqwest::Client, reqwest::Error>`
    (redirects off, `timeout`, `user_agent`) and `archive::http_client(cfg: &Config) ->
    Result<reqwest::Client, reqwest::Error>` (today's archive settings, default redirects).
  - `run` builds both after `config::load` and before `check_pages`. Either error prints
    `rotguard: cannot load the system's CA certificates: <error chain>` (the design's wording,
    using the existing `chain` helper for the cause) and returns exit 2.
  - `check_pages` passes the checker client to `check_all(urls, client, cfg, …)`, which hands it
    to `Fetcher`; `look_up_snapshots` gets the archive client and passes it to
    `Archive::new(client, cfg)`. No `expect` remains on `Client::builder().build()`.
  - Unit tests: the `Fetcher` helper (`src/check.rs` ~777), the 13 `check_all` calls in
    `src/check.rs` tests and the `Archive` helper (`src/archive.rs` ~312) build their client
    with `.expect`.
- Test: un-pends S14 (proved in Docker, see the header); existing unit tests keep passing.
- Covers: R7, S14
- Commit: Exit cleanly without CA certificates

### T3: Verify a release tag
- Files: scripts/release-verify.sh
- Interfaces: `scripts/release-verify.sh <notes file>`, run from the repo root, reads
  `GITHUB_REF_NAME`. Reads `version = "…"` from the `[package]` table of `Cargo.toml`; if
  `GITHUB_REF_NAME` isn't `v<version>`, prints `release-verify: tag <tag> does not match
  Cargo.toml version <version>` to stderr and exits 1. Writes the lines after `## [<version>]`
  (any suffix such as ` - 2026-11-05`) up to the next `## ` line, with leading and trailing blank
  lines trimmed, to the notes file; with no such heading, prints `release-verify: CHANGELOG.md
  has no ## [<version>] section` and exits 1 without writing the file. `set -euo pipefail`,
  executable, passes `shellcheck`.
- Test: un-pends S1, S2, S3.
- Covers: R1, R2, S1, S2, S3
- Commit: Verify a release tag against Cargo.toml

### T4: Publish a release
- Files: scripts/release-publish.sh
- Interfaces: `scripts/release-publish.sh <tag> <dist dir> <notes file>`, reads `GITHUB_SHA`.
  Runs the design's 5 steps in order (Details, "Release workflow"), `cd`ing into the dist dir for
  `sha256sum rotguard-* > SHA256SUMS` so the file holds bare names. `<major>` is the digits
  between `v` and the first `.` of the tag. If `gh release upload` or `gh release edit` fails, it
  runs `gh release delete <tag> --yes` and exits non-zero; no `git` command runs unless step 4
  succeeded. `set -euo pipefail`, executable, passes `shellcheck`.
- Test: un-pends S5, S6, S17.
- Covers: R1, R3, S5, S6, S17
- Commit: Publish a release and move its major tag

### T5: Release workflow
- Files: .github/workflows/release.yml
- Interfaces: The 3 jobs in the design's Details sketch, exactly: trigger
  `on: push: tags: ['v[0-9]+.[0-9]+.[0-9]+']` only; top-level `permissions: contents: read`;
  `verify` (`actions/checkout@v7`, `scripts/release-verify.sh notes.md`, rustup stable,
  `cargo test --locked`, `actions/upload-artifact` name `notes`); `build` with `needs: verify`
  and a 2-entry matrix (`ubuntu-latest` + `x86_64-unknown-linux-musl`, `ubuntu-24.04-arm` +
  `aarch64-unknown-linux-musl`) that installs `musl-tools`, builds with `--locked --target`,
  `install -m 755`s to `dist/rotguard-<X.Y.Z>-<target>`, checks
  `--version` equals `rotguard <X.Y.Z>` (version from `${GITHUB_REF_NAME#v}`), and uploads that
  file; `publish` with `needs: build`, job-level `permissions: contents: write`, checkout,
  `actions/download-artifact` (`pattern: rotguard-*`, `merge-multiple: true`, `path: dist`, and
  `name: notes`, `path: notes`), then `scripts/release-publish.sh "$GITHUB_REF_NAME" dist
  notes/notes.md` with `env: GH_TOKEN: ${{ github.token }}`. Use the action major versions
  `ci.yml` and `action.yml` already use where they overlap (`actions/checkout@v7`); pick current
  majors for `upload-artifact` and `download-artifact`. `${{ }}` only in `env:`/`with:`, never
  inside a `run:` script.
- Test: un-pends S4.
- Covers: R1, R2, R3, S4
- Commit: Add the release workflow

### T6: Download the release binary in the action
- Files: action.yml, tests/acceptance/build_cache.rs
- Interfaces: A first step `id: fetch`, `shell: bash`, per the design's Details "Action fetch
  step": ref `${GITHUB_ACTION_PATH#*/_actions/*/*/}`, release mode only for
  `^v[0-9]+(\.[0-9]+\.[0-9]+)?$` on `RUNNER_OS=Linux` with `RUNNER_ARCH` `X64` (`x86_64`) or
  `ARM64` (`aarch64`); writes `mode=release` or `mode=build` to `GITHUB_OUTPUT`. In release mode:
  print `Download rotguard <version> (<arch>)`; `curl --fail --location --retry 3 --silent
  --show-error --output <file> <url>` once for the asset and once for `SHA256SUMS`, asset first,
  into `$RUNNER_TEMP`; check with `grep -F " <asset>" SHA256SUMS | sha256sum -c -` from
  `$RUNNER_TEMP`; `mkdir -p` then `install -m 755` to
  `$GITHUB_ACTION_PATH/target/release/rotguard`. Every failure point is guarded with
  `|| { echo "::error::rotguard: <what> (<url or asset>)"; exit 1; }` so the message prints under
  `bash -e`. The key, cache and build steps get `if: steps.fetch.outputs.mode == 'build'`, the
  save step `if: steps.fetch.outputs.mode == 'build' && steps.cache.outputs.cache-hit != 'true'`;
  the run step is unchanged. Never write the runner's home path; the privacy check flags it.
  `build_cache.rs` is locked (build-cache S1): open it with `whittle-gates amend`, and in
  `build_cache_s1_…` change only the step count to 6, each index by +1, and the save `if` to
  the string above; the design approves this.
- Test: un-pends S7, S8, S9, S10, S11, S12, S13, S16, S18; the amended build-cache S1 passes.
- Covers: R4, R5, R6, S7, S8, S9, S10, S11, S12, S13, S16, S18
- Commit: Download the release binary in the action

### T7: Docs
- Files: CLAUDE.md, README.md, CONTRIBUTING.md, CHANGELOG.md, docs/github-actions.md, docs/architecture.md, docs/config.md, tests/acceptance/action.rs, tests/cli.rs
- Interfaces: Read the branch's diff against the `Base:` in `.whittle/run/progress.md` first.
  - `docs/github-actions.md`: the yaml block uses `rdiachenko/rotguard@v0`; the paragraph after
    it says a `@v0` site downloads a prebuilt binary and other refs build with the cache; the
    outcomes row "build or fetch fails" covers a failed download.
  - `tests/acceptance/action.rs` is locked: open it with `whittle-gates amend` and change only
    `action_s5`'s `rdiachenko/rotguard@main` to `rdiachenko/rotguard@v0` (S15; the design
    approves it).
  - `CLAUDE.md` (new): a short "Releasing" section an agent follows: the version rule (D12:
    patch fixes, minor adds, major for any breaking change, 0.x included); bump `Cargo.toml`,
    run `cargo build` for `Cargo.lock`, rename `## [Unreleased]` to `## [X.Y.Z] - <date>` with a
    new empty `## [Unreleased]` above (D10); run the gates; commit `Release X.Y.Z`; push `main`
    and wait for CI; `git tag vX.Y.Z && git push origin vX.Y.Z`; watch with `gh run watch`;
    check the release's 3 assets and that `v<major>` moved. Recovery: a failure before publish
    leaves no release, so delete the tag (`git push --delete origin vX.Y.Z`, `git tag -d`), fix,
    and tag again; a published release is final, so fix forward with the next patch. First
    release only: make the repo public, enable immutable releases and private vulnerability
    reporting, then after v0.1.0 move the blog to `@v0` and check its run log shows
    `Download rotguard 0.1.0 (x86_64)` (confirms the folder-name fact and `if:` on composite
    steps) and that the second release moved `v0` (confirms the force-push fact).
  - `CONTRIBUTING.md`: 1 line pointing maintainers to `CLAUDE.md` for releases.
  - `README.md`: Install adds downloading a binary from the releases page; the GitHub Actions
    pointer says to pin `@v0`.
  - `CHANGELOG.md`: under `## [Unreleased]`, add prebuilt Linux binaries, the `@v0` action
    pin, and the clean exit without CA certificates.
  - `docs/config.md`: the `user_agent` default and the example stop naming `0.1.0`
    (`rotguard/<version>`), so a release doesn't need to edit it (D9).
  - `tests/cli.rs`: compare `--version` with `concat!("rotguard ", env!("CARGO_PKG_VERSION"))`.
  - `docs/architecture.md`: add the release workflow and GitHub Releases to Context and
    Components, the fetch step to the action's steps and the Weekly fix PR flow, the CA-bundle
    exit to the Check flow's exit-2 list, a Hard parts entry for the action's ref detection, and
    Decisions lines for D1, D2 and D4 of `docs/design/release-binary.md`; reword the
    build-from-source decision line to say it applies to non-tag refs.
- Test: none new; S15 is the amended `action_s5`.
- Covers: R8, S15
- Review: skip
- Commit: Update docs for release binaries

## Verification contract
```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'
! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'
scripts/privacy-check.sh --all
```
