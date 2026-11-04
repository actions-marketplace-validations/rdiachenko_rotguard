---
status: approved
---
# Release binaries and a pinned action

## Overview

### Summary
Pushing a `vX.Y.Z` tag builds static Linux binaries for x86_64 and arm64 and publishes them as a
GitHub release, with that version's CHANGELOG section as the notes. The action then downloads
the release binary when a site pins `@v0` or `@v0.1.0`, and builds from source as today for
`@main`, a branch or a SHA. rotguard also stops panicking on a machine with no CA certificates.
Size ~390 lines; no questions wait under Open for you.

### Problem
The blog runs `rdiachenko/rotguard@main`, so every Monday it runs whatever `main` holds that
morning, and there is no version a site can stay on while `main` changes. Once rotguard is
public, anyone who copies `docs/github-actions.md` gets the same moving target.

Every run also compiles rotguard on the site's runner. With the build cache that costs 22–27 s
(blog runs of 2026-10-26 and 2026-11-02); on a cache miss it costs about 2.5 min (219 s for the
whole action on 2026-10-21). A miss comes with every new stable Rust, about every 6 weeks, every
`Cargo.lock` change, and every site's first run.

### What you can do with it
1. Cut a release: bump the version and the CHANGELOG heading, commit, then
   `git tag v0.2.0 && git push origin v0.2.0`. About 8 minutes later
   `github.com/rdiachenko/rotguard/releases/tag/v0.2.0` holds
   `rotguard-0.2.0-x86_64-unknown-linux-musl`, `rotguard-0.2.0-aarch64-unknown-linux-musl` and
   `SHA256SUMS`, and `v0` points at v0.2.0. `CLAUDE.md` lists the steps, and I run them for you.
2. Pin a site with `uses: rdiachenko/rotguard@v0`. The run log shows
   `Download rotguard 0.2.0 (x86_64)` and the action spends about 3 s getting rotguard instead of
   22 s or more.
3. Try an action change on the blog before releasing it with `uses: rdiachenko/rotguard@main`
   or `@<branch>`: that path builds from source with the cache, exactly as today.
4. Install the CLI without Rust: download the binary for your machine from the releases page.

### Not in this version
- macOS and Windows binaries: no runner uses them. A macOS runner builds from source.
- Signature checks such as `gh attestation verify`: they need a token and an API call on every
  run. The action checks the sha256, and GitHub's immutable releases stop anyone replacing an
  asset after publishing.
- Pre-release tags (`v0.2.0-rc.1`): a run from a branch is the preview.
- Publishing to crates.io, Homebrew or another package manager.
- A CI job that runs the action itself on a fixture site.

### How it works
Two pieces are new: `.github/workflows/release.yml` in this repo, and a first `fetch` step in
`action.yml`. The run step and the site workflow's PR logic don't change; the binary changes
only in how it reports a missing CA bundle.

```text
Context: releasing and running rotguard

  You ──git push origin vX.Y.Z──► release.yml [GitHub Actions, rotguard repo]
                                   │ verify, build x86_64 + aarch64, publish, move v0
                                   ▼
                         GitHub Releases ◄──GET binary + SHA256SUMS (HTTPS, no token)──┐
                         (immutable)                                                  │
  site workflow ──uses: rdiachenko/rotguard@v0──► action.yml [composite, site runner] ┘
                                                   │ ref not a v-tag: build from source
                                                   ▼
                                         rotguard --apply ──► report, PR
```

#### The release workflow
A tag push matching `v[0-9]+.[0-9]+.[0-9]+` runs 3 jobs in order. Verify checks the tag equals
`v` plus the Cargo.toml version, finds a `## [X.Y.Z]` section in `CHANGELOG.md` for the notes,
and runs the tests. Build compiles `--target <arch>-unknown-linux-musl` on `ubuntu-latest` and
`ubuntu-24.04-arm` and runs each binary's `--version`. Publish creates a draft release, uploads
the 2 binaries and `SHA256SUMS`, publishes it, and only then moves `v0`.

Immutable releases make a published tag final: its assets can't change and its name can't be
reused. So every check runs before publishing, and a release found broken afterwards is fixed by
the next patch version. A failure before publishing leaves no release and `v0` where it was: if
the upload fails after the draft exists, the publish script deletes the draft. `CLAUDE.md` then
says to delete the tag, fix, and push the tag again.

#### How the action picks its binary
The runner unpacks each action into `…/_actions/<owner>/<repo>/<ref>`, with the ref exactly as
written in its `uses:` line: `…/_actions/rdiachenko/rotguard/main` in the blog's runs. A branch with
a slash, such as `release/v0.2.0`, becomes nested folders. So the fetch step takes the ref as
everything after `_actions/<owner>/<repo>/`, not the last folder name. GitHub's `github.action_ref`
looks like the obvious source, but inside an action that another composite action calls it holds the
caller's ref (actions/runner#2473), so a wrapper at `@v1` calling rotguard at `@main` would download
a release.

When the ref looks like a release tag, `v0` or `v0.1.0` but not `v0-fix` or `release/v0.2.0`, and
the runner is Linux on X64 or ARM64, the step downloads the binary for the version in the action's
own `Cargo.toml`. `v0` always points at a release commit, so that version always has a release. Any
other ref or platform gets `build`, and the 4 cache and build steps run as today. The step judges
the ref's name only, so a branch named exactly like a release tag, such as `v0.2.0`, counts as one;
don't name branches that way.

#### When the download fails
A 404, a network error or a checksum mismatch fails the job with an error naming the URL, and
leaves no binary behind. It doesn't fall back to building: a 404 means the release process
broke, and a bad checksum must never pass quietly. The site's existing alert fires, as it does
today for a failed build.

If the folder path turns out not to hold the tag for some ref style, that ref builds from source:
slower, never wrong.

#### No CA certificates
On a system without a CA bundle, rotguard panics today with a message about `user_agent` and
exits 101. It will print `rotguard: cannot load the system's CA certificates: …` and exit 2,
the code for "rotguard itself failed". A downloaded static binary makes slim containers, where
this happens, more likely.

**Main flow** (`@v0` on ubuntu-latest):
1. fetch: ref `v0`, Linux X64, `Cargo.toml` says 0.2.0, so it downloads
   `rotguard-0.2.0-x86_64-unknown-linux-musl` and `SHA256SUMS`, checks the sum and installs the
   binary at `$GITHUB_ACTION_PATH/target/release/rotguard`.
2. key, cache, build and save are skipped.
3. run: `rotguard --apply --format markdown`, unchanged.

**Main failure**: the download returns 404, so fetch prints the error and exits 1. The job fails
and the site's notify job alerts. No PR step runs.

### Decisions
#### D1: A tag ref downloads, any other ref builds from source
- Why: `@v0` gets a fixed, tested binary in about 3 s, while `@main` and branches keep today's
  path, so an action change can run on the blog before it's released. You chose it over the 2
  options below.
- Rejected: always download the Cargo.toml version. `@main` would run the last release's binary
  and hit a 404 between a version bump and its tag.
- Rejected: tag only, other refs fail. There'd be no way to try a change on a real site without
  cutting a release.
- Limit: a ref is classified by its name, so a branch named exactly like a release tag
  (`v0.2.0`) is treated as one: it downloads the release for its `Cargo.toml` version, or fails
  with a 404 when that version has none. A branch with a slash never matches.
- Cost if wrong: if nobody uses a non-tag ref, about 30 lines of `action.yml` and
  `tests/acceptance/build_cache.rs` stay as dead weight; removing them takes an hour.

#### D2: Read the ref from the action's folder path
- Why: the runner unpacks each action into `_actions/<owner>/<repo>/<ref>`, with the ref from its
  own `uses:` line unchanged (`ActionManager.cs`; probed for `@main`), so nesting can't change
  it. The step takes everything after `_actions/<owner>/<repo>/`, so a slashed branch such as
  `release/v0.2.0` stays whole and builds. `action.yml` already
  relies on `$GITHUB_ACTION_PATH` everywhere.
- Rejected: `github.action_ref` through `env:`. Inside an action called by another composite
  action it holds the caller's ref (actions/runner#2473), which picks the wrong mode.
- Rejected: an `inputs.version` the site sets. It duplicates the pin and can disagree with it.
- Cost if wrong: if the folder path doesn't hold the tag for tag refs, every `@v0` site builds
  from source: slower, never wrong. The first `@v0` run's log shows it.

#### D3: Static musl binaries, a bare file per arch plus `SHA256SUMS`
- Why: probed. Both targets build with only `musl-tools` (aws-lc-sys included), run on Ubuntu
  24.04 and check real HTTPS links through the system CA store. They don't depend on the
  runner's glibc.
- Rejected: `-gnu` binaries. They tie each release to the glibc of the runner that built it.
- Cost if wrong: switching targets is a few lines in `release.yml` and the asset names.

#### D4: Publish, then move `v0`, and never rewrite a published tag
- Why: with immutable releases on, a published release's tag and assets are final, so the
  workflow publishes only after every check, and moves `v0` last so `@v0` never points at a
  half-made release.
- Rejected: `softprops/action-gh-release`. The preinstalled `gh` does the same with no
  third-party code holding a write token.
- Cost if wrong: if `v0` can't be force-pushed once immutable releases are on, `v0` becomes a
  branch fast-forwarded to each release; `uses:` accepts branches, so sites don't change.

#### Smaller decisions
- D5: the fetch step downloads from `https://github.com/rdiachenko/rotguard/releases/download/`
  with `curl --fail --location --retry 3`, because release URLs on a public repo need no token.
  A fork edits that 1 line.
- D6: the binary goes to `$GITHUB_ACTION_PATH/target/release/rotguard`, where the build puts it,
  so the run step and its tests stay as they are.
- D7: `docs/github-actions.md` pins `@v0` and keeps its `notify` job; its notes already say how
  to replace or drop it.
- D8: the release procedure lives in `CLAUDE.md`, as you asked, and `CONTRIBUTING.md` points to
  it in 1 line.
- D10: `CHANGELOG.md` keeps Keep a Changelog's form, which it already states: a release renames
  `## [Unreleased]` to `## [X.Y.Z] - <date>` and adds an empty `## [Unreleased]` above it.
  You chose it.
- D11: each major version has its own moving tag: v1.0.0 starts `v1`, and `v0` stays on the last
  0.x. You chose it, so a site on `@v0` never jumps across a breaking release.
- D12: a breaking change (a renamed or removed `rotguard.toml` key or flag, a changed exit code,
  action output or JSON field) gets a new major version, and 0.x counts as a major version too:
  within 0.x a release only fixes or adds. `CLAUDE.md` states the rule, because `@v0` follows
  every 0.x release.
- D9: docs stop naming the current version where a release would have to edit them
  (`docs/config.md`), and `tests/cli.rs` compares against `CARGO_PKG_VERSION`.

### Assumptions
- A1: You enable "Immutable releases" and make the repo public before the first tag. Both are in
  the `CLAUDE.md` first-release steps, and I'll do them with you.
- A2: The first release is v0.1.0, today's version, cut after this merges, and the blog moves to
  `@v0` after it publishes.

### Scope
- `.github/workflows/release.yml` with `scripts/release-verify.sh` and
  `scripts/release-publish.sh`: ~110 lines, to cut releases.
- `action.yml` fetch step and 4 `if:` conditions: ~30 lines, for `@v0` sites.
- Clean exit without CA certificates: ~20 lines in `src/check.rs`, `src/archive.rs` and
  `src/lib.rs`, for anyone running the binary in a slim container.
- Locked tests that change: build-cache S1 (`build_cache_s1_…`) expects 6 steps with `fetch`
  first, every index shifted by 1, and the save `if` from S9; weekly-fix-pr S5 (`action_s5_…`)
  expects `rdiachenko/rotguard@v0` instead of `@main`. Approving this design approves both.
- Docs: ~70 lines. `CLAUDE.md` release procedure, `README.md` install from a release,
  `docs/github-actions.md` `@v0`, `docs/architecture.md`, `docs/config.md`, `CONTRIBUTING.md`.

Size: ~390 (230 estimated × 1.7, median of the last 5 cross-repo design builds)
Touches: `.github/**`, `action.yml` and `scripts/**` — the release workflow, its 2 scripts, and the
action's fetch step.

### Open for you
none

## Spec

### Requirements
- R1: WHEN a tag matching `v[0-9]+.[0-9]+.[0-9]+` is pushed THE release workflow SHALL publish a
  GitHub release for that tag with the assets `rotguard-<X.Y.Z>-x86_64-unknown-linux-musl`,
  `rotguard-<X.Y.Z>-aarch64-unknown-linux-musl` and `SHA256SUMS`, and the body of the
  `## [X.Y.Z]` section of `CHANGELOG.md` as its notes.
- R2: IF the tag is not `v` followed by the `Cargo.toml` version, or `CHANGELOG.md` has no
  `## [X.Y.Z]` heading, or `cargo test --locked` fails, or a binary's `--version` doesn't print
  `rotguard X.Y.Z`, THEN the release workflow SHALL publish nothing and leave `v<major>`
  unchanged.
- R3: WHEN the release is published THE release workflow SHALL point the tag `v<major>` at the
  released commit, and not before.
- R4: WHEN the action's ref, the part of `$GITHUB_ACTION_PATH` after `_actions/<owner>/<repo>/`,
  matches `^v[0-9]+(\.[0-9]+\.[0-9]+)?$` on a runner with `RUNNER_OS=Linux` and `RUNNER_ARCH` `X64`
  or `ARM64` THE action SHALL download the asset for the action's `Cargo.toml` version and the
  runner's arch, verify it against `SHA256SUMS`, install it executable at
  `$GITHUB_ACTION_PATH/target/release/rotguard`, and skip the key, cache, build and save steps.
- R5: IF the download of the asset or `SHA256SUMS` fails, or the checksum doesn't match, THEN
  the action SHALL fail before the run step with an `::error::` line naming the URL or asset,
  and leave no file at `$GITHUB_ACTION_PATH/target/release/rotguard`.
- R6: WHEN the action's ref, the part of `$GITHUB_ACTION_PATH` after `_actions/<owner>/<repo>/`,
  doesn't match `^v[0-9]+(\.[0-9]+\.[0-9]+)?$`, or the runner is not Linux on X64 or ARM64, THE
  action SHALL run the key, cache, build and save steps as before this change, and download nothing.
- R7: IF the HTTP client can't be built because no CA certificates load THEN rotguard SHALL
  print a line starting `rotguard: ` to stderr and exit 2, without a panic.
- R8: THE documented site workflow in `docs/github-actions.md` SHALL use
  `rdiachenko/rotguard@v0`.

### Acceptance scenarios
- S1 [R1]: Given `Cargo.toml` at 0.1.0 and a `CHANGELOG.md` whose `## [0.1.0] - 2026-11-05`
  section holds 2 bullets followed by `## [0.0.9]`, when the release verify script runs with
  `GITHUB_REF_NAME=v0.1.0`, then it exits 0 and the notes file holds exactly those 2 bullets.
- S2 [R2]: Given the same repo, when the verify script runs with `GITHUB_REF_NAME=v0.1.1`, then
  it exits non-zero and prints an error naming `v0.1.1` and `0.1.0`.
- S3 [R2]: Given `Cargo.toml` at 0.1.0 and a `CHANGELOG.md` with only `## [Unreleased]`, when the
  verify script runs with `GITHUB_REF_NAME=v0.1.0`, then it exits non-zero and prints an error
  naming `## [0.1.0]`.
- S4 [R1, R2, R3]: Given `.github/workflows/release.yml`, then it triggers only on push of tags
  matching `v[0-9]+.[0-9]+.[0-9]+`; the verify job runs `cargo test --locked`; the build job
  needs verify and has 2 matrix entries, `ubuntu-latest` with `x86_64-unknown-linux-musl` and
  `ubuntu-24.04-arm` with `aarch64-unknown-linux-musl`, each running the built binary's
  `--version` against the tag; the publish job needs build; only the publish job has
  `contents: write`.
- S5 [R1, R3]: Given a dist folder with both binaries and stub `gh` and `git` that log their
  arguments, when the publish script runs for `v0.1.0`, then the log shows in order:
  `gh release create v0.1.0 --draft --verify-tag` with the notes file, an upload of both
  binaries and `SHA256SUMS`, `gh release edit v0.1.0 --draft=false`, then a forced tag `v0` and
  a forced push of `v0`; and `SHA256SUMS` lists both assets with their real sha256.
- S6 [R2, R3]: Given a stub `gh` that fails on `release create`, when the publish script runs,
  then it exits non-zero and the log has no `git` call.
- S17 [R2]: Given a stub `gh` that succeeds on `release create` and fails on `release upload`,
  when the publish script runs for `v0.1.0`, then it exits non-zero, the log's last `gh` call is
  `gh release delete v0.1.0 --yes`, and the log has no `git` call.
- S7 [R4]: Given `GITHUB_ACTION_PATH` ending in `/_actions/o/r/v0` with a `Cargo.toml` at
  0.1.0, `RUNNER_OS=Linux`, `RUNNER_ARCH=X64` and a stub `curl` that serves a fake binary and a
  matching `SHA256SUMS`, when the fetch step runs, then it exits 0, `GITHUB_OUTPUT` has
  `mode=release`, `target/release/rotguard` is the fake binary and executable, and the stub saw a
  URL ending in
  `rdiachenko/rotguard/releases/download/v0.1.0/rotguard-0.1.0-x86_64-unknown-linux-musl`.
- S8 [R4]: Given the S7 setup with `RUNNER_ARCH=ARM64` and the path ending in
  `/_actions/o/r/v0.1.0`, when the
  fetch step runs, then the stub saw the `aarch64-unknown-linux-musl` asset URL and
  `GITHUB_OUTPUT` has `mode=release`.
- S9 [R4, R6]: Given `action.yml`, then its first step has `id: fetch`; the key, cache and build
  steps have `if: steps.fetch.outputs.mode == 'build'`; the save step has
  `if: steps.fetch.outputs.mode == 'build' && steps.cache.outputs.cache-hit != 'true'`; and the
  run step has no `if`.
- S10 [R5]: Given the S7 setup with a `SHA256SUMS` whose sum doesn't match the binary, when the
  fetch step runs, then it exits non-zero, prints a line starting `::error::` naming the asset,
  and `target/release/rotguard` doesn't exist.
- S11 [R5]: Given the S7 setup with a stub `curl` that exits 22 (HTTP 404), when the fetch step
  runs, then it exits non-zero, prints a line starting `::error::` with the asset URL, and
  `target/release/rotguard` doesn't exist.
- S12 [R6]: Given `GITHUB_ACTION_PATH` ending in `/_actions/o/r/main`, Linux and X64, when the fetch
  step
  runs, then it exits 0, `GITHUB_OUTPUT` has `mode=build`, and the stub `curl` was never called.
- S16 [R6]: Given `GITHUB_ACTION_PATH` ending in `/_actions/o/r/v0-fix`, Linux and X64, when the
  fetch step runs, then it exits 0, `GITHUB_OUTPUT` has `mode=build`, and the stub `curl` was
  never called.
- S18 [R6]: Given `GITHUB_ACTION_PATH` ending in `/_actions/o/r/release/v0.2.0`, Linux and X64,
  when the fetch step runs, then it exits 0, `GITHUB_OUTPUT` has `mode=build`, and the stub
  `curl` was never called.
- S13 [R6]: Given `GITHUB_ACTION_PATH` ending in `/_actions/o/r/v0` with `RUNNER_OS=macOS`, when the
  fetch
  step runs, then `GITHUB_OUTPUT` has `mode=build` and the stub `curl` was never called.
- S14 [R7]: Given Linux, a site with 1 external link, and `SSL_CERT_FILE` and `SSL_CERT_DIR`
  pointing at an empty folder, when rotguard runs, then it exits 2, stderr starts with
  `rotguard: ` and doesn't contain `panicked`.
- S15 [R8]: Given `docs/github-actions.md`, then the `id: rotguard` step of its first yaml block
  uses `rdiachenko/rotguard@v0`. This is weekly-fix-pr S5 (`action_s5_…`) with its `@main`
  assertion changed to `@v0`, not a new test.

### Details

**Release workflow** (`.github/workflows/release.yml`):

```text
on: push: tags: ['v[0-9]+.[0-9]+.[0-9]+']       permissions: contents: read (top level)
verify  [ubuntu-latest]  checkout; scripts/release-verify.sh notes.md; cargo test --locked
        upload-artifact name: notes, path: notes.md
build   [matrix]  needs: verify
          ubuntu-latest     x86_64-unknown-linux-musl
          ubuntu-24.04-arm  aarch64-unknown-linux-musl
        checkout; apt-get install musl-tools; rustup target add T
        cargo build --release --locked --target T
        install -m 755 target/T/release/rotguard dist/rotguard-<X.Y.Z>-T
        test "$(dist/rotguard-<X.Y.Z>-T --version)" = "rotguard <X.Y.Z>"
        upload-artifact name: rotguard-<X.Y.Z>-T, path: dist/rotguard-<X.Y.Z>-T
publish [ubuntu-latest]  needs: build   permissions: contents: write
        checkout (default depth; its credentials let git push the tag)
        download-artifact pattern: rotguard-*, merge-multiple: true, path: dist
        download-artifact name: notes, path: notes
        env GH_TOKEN: ${{ github.token }}
        scripts/release-publish.sh "$GITHUB_REF_NAME" dist notes/notes.md
```

The verify and publish logic lives in `scripts/release-verify.sh` and
`scripts/release-publish.sh`, so the acceptance tests run them with stubs, the way
`build_cache.rs` runs the key step. `scripts/release-verify.sh <notes file>` runs from the repo
root: it reads the version from `Cargo.toml`'s `[package]` table, compares it with
`GITHUB_REF_NAME`, and writes the lines between `## [X.Y.Z]` and the next `## ` heading, trimmed
of blank edges, to the notes file. Errors go to stderr with exit 1, and no notes file is written.
`scripts/release-publish.sh <tag> <dist dir> <notes file>` expects `dist` to hold exactly the 2
`rotguard-<X.Y.Z>-<target>` files.
`release-publish.sh` runs, in order and stopping at the first failure:
1. `sha256sum rotguard-* > SHA256SUMS` in the dist folder.
2. `gh release create <tag> --draft --verify-tag --title <tag> --notes-file <notes>`.
3. `gh release upload <tag> <both binaries> SHA256SUMS`.
4. `gh release edit <tag> --draft=false`.
5. `git tag -f v<major> "$GITHUB_SHA"` and `git push -f origin v<major>`, where `<major>` is
   the first number of the version. A lightweight tag needs no git identity.

If step 3 or 4 fails, the script runs `gh release delete <tag> --yes` before exiting non-zero,
so no draft is left behind.

**Action fetch step** (`id: fetch`, `shell: bash`, first in `action.yml`; no `${{` in the
script):

| Ref | `RUNNER_OS` | `RUNNER_ARCH` | Output | Downloads |
|---|---|---|---|---|
| release tag | `Linux` | `X64` | `mode=release` | `…-x86_64-unknown-linux-musl` |
| release tag | `Linux` | `ARM64` | `mode=release` | `…-aarch64-unknown-linux-musl` |
| release tag | anything else | anything else | `mode=build` | nothing |
| anything else | any | any | `mode=build` | nothing |

The ref is `${GITHUB_ACTION_PATH#*/_actions/*/*/}`, everything after `_actions/<owner>/<repo>/`.
A release tag matches `^v[0-9]+(\.[0-9]+\.[0-9]+)?$`, so a ref with a slash never does, and a
path without `/_actions/` keeps its full text and builds. The script holds no `${{`, as
`action_s4` requires.

In release mode it reads the version from `$GITHUB_ACTION_PATH/Cargo.toml`, downloads the asset
and `SHA256SUMS` into `$RUNNER_TEMP`, checks the asset's line with `sha256sum -c`, then
`install -m 755`s it to `$GITHUB_ACTION_PATH/target/release/rotguard`. It prints
`Download rotguard <version> (<arch>)` first. Any failure prints
`::error::rotguard: <what> (<url or asset>)` and exits 1; nothing is installed until the check
passes. The test stub replaces `curl` on `PATH`. Test assertions match the URL's
`github.com/…` suffix rather than a full `https://` literal, so the fitness rule that bans real
hosts in `tests/` still holds.

**CA certificates**: `Fetcher::new` and the archive client stop calling `expect` on
`Client::builder().build()`. `run` builds the clients before checking and maps the error to
`rotguard: cannot load the system's CA certificates: <cause>` (or the reqwest error text for any
other builder failure), exit 2. S14 runs only on Linux (`#[cfg(target_os = "linux")]`); macOS
reads its certificates from the keychain, not these variables.

**Facts**:
- probed: `cargo build --release --locked --target x86_64-unknown-linux-musl` in `rust:1`
  Docker with `musl-tools`, 120 s, 12 MB static-pie; runs on `ubuntu:24.04` and checks 2 real
  HTTPS links.
- probed: the same for `aarch64-unknown-linux-musl` on arm64 Docker, 37 s, static; runs and
  checks the same links.
- probed: the blog's runs unpack the action at `…/_actions/rdiachenko/rotguard/main`. Tag refs:
  unverified — fallback: a non-matching ref builds from source (R6).
- read from source, not probed: a ref with a slash becomes nested folders
  (`ActionManager.cs` joins the actions folder, `owner/repo` and the ref unchanged) — fallback: a
  ref that doesn't match the tag pattern builds (R6).
- probed: with `SSL_CERT_FILE` and `SSL_CERT_DIR` pointing at an empty folder, the static binary
  panics at `src/check.rs:104` and exits 101, so S14 reproduces the missing-bundle case.
- unverified — fallback in D4: `v0` stays force-pushable with immutable releases on.
- unverified: `if:` on a composite step reading `steps.fetch.outputs.mode` (documented in the
  action metadata syntax); the first `@v0` run confirms it.

**Invariants**:
- Nothing is installed at `target/release/rotguard` before its checksum passes (fetch step).
- `v<major>` moves only after `gh release edit --draft=false` succeeds, and a failed publish
  leaves no draft (`release-publish.sh`).
- Only the publish job holds a token that can write (`release.yml`).

**Budgets**: release about 8 minutes (cold `cargo test` unmeasured, about 3 min; 2 musl builds
in parallel, measured 120 s and 37 s in Docker; publish under 30 s). Fetch about 3 s for 12 MB,
unmeasured until the first `@v0` run.

### Risk paths
`.github/**`, `action.yml`, `scripts/**` (already in `.whittle/gates.toml`).

### Fitness rules
No change: the 3 rules in `.whittle/gates.toml` still apply. The `tests/` host rule shapes S7's
URL assertion. `scripts/privacy-check.sh` flags any `/home/<name>` path, so `action.yml`, docs and
tests never write the runner's home path; write `…/_actions/…` or use `$GITHUB_ACTION_PATH`.

### Review changes
- Round 1: a release tag is `^v[0-9]+(\.[0-9]+\.[0-9]+)?$`, so a branch such as `v0-fix` builds
  from source (new S16).
- Round 1: the publish script deletes the draft when an upload fails after it was created (new S17).
- Round 1: D2 corrected: `github.action_ref` via `env:` is the first source, the folder name the
  fallback.
- Round 1: the runner home path is written as `…/_actions/…` so the privacy check passes.
- Round 1: Scope names the 2 locked tests that change (build-cache S1, weekly-fix-pr S5); S15 is
  the changed S5.
- Round 1: the publish job runs `actions/checkout` and sets `GH_TOKEN`.
- Round 1: the build job uploads `dist/rotguard-<X.Y.Z>-<target>`; publish merges them into `dist`
  and downloads the notes separately.
- Round 1: the overview says the binary changes only in the CA-bundle error.
- Round 1: `release-verify.sh` takes the notes path and runs from the repo root.
- Round 2: the ref comes only from the action's folder name; `github.action_ref` is rejected
  because a wrapping composite action passes its own ref (D2, R4).
- Round 2: D1 states that refs are classified by name, so a branch named like a release tag
  counts as one.
- Round 2: S16 is now a `v0-fix` folder that builds without calling `curl`.
- Round 3: the ref is everything after `_actions/<owner>/<repo>/`, not the last folder name, so a
  slashed branch such as `release/v0.2.0` builds (new S18); fixtures include `_actions/o/r/`.
