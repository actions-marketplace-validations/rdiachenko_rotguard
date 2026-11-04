# rotguard

See `docs/architecture.md` for the code and `CONTRIBUTING.md` for the checks.

## Releasing

Pushing a `vX.Y.Z` tag runs `.github/workflows/release.yml`: it verifies the tag, builds static
Linux binaries for x86_64 and arm64, publishes a GitHub release with that version's CHANGELOG
section as the notes, and moves the `v<major>` tag. Sites pinned to `@v0` follow every 0.x
release, so pick the version by this rule: a patch fixes, a minor adds, and any breaking change
(a renamed or removed `rotguard.toml` key or flag, a changed exit code, action output or JSON
field) is a new major version. 0.x counts as a major version: within it a release only fixes or
adds.

1. Set `version` in `Cargo.toml` to X.Y.Z and run `cargo build` to update `Cargo.lock`.
2. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [X.Y.Z] - <date>` and add an empty
   `## [Unreleased]` above it.
3. Run the checks in `CONTRIBUTING.md`.
4. Commit as `Release X.Y.Z`, push `main` and wait for CI to pass.
5. `git tag vX.Y.Z && git push origin vX.Y.Z`
6. `gh run watch` until the release run ends.
7. `gh release view vX.Y.Z` shows 3 assets: the 2 binaries and `SHA256SUMS`. Check that
   `git ls-remote --tags origin v<major>` shows the new commit.

A failure before publishing leaves no release and `v<major>` where it was. Delete the tag with
`git push --delete origin vX.Y.Z && git tag -d vX.Y.Z`, fix the cause, and tag again. A published
release is final (immutable releases): fix forward with the next patch version.

The arm64 build assumes the `ubuntu-24.04-arm` image ships rustup. If that job fails, nothing
publishes.

### First release only

1. Ask the user, then make the repo public: `gh repo edit rdiachenko/rotguard --visibility public
   --accept-visibility-change-consequences`. It is outward-facing.
2. Turn on immutable releases (repo Settings, General, Releases) and private vulnerability
   reporting: `gh api -X PUT repos/rdiachenko/rotguard/private-vulnerability-reporting`.
3. After v0.1.0 publishes, switch the blog
   (`../rdiachenko.github.com/.github/workflows/rotguard.yml`) to `@v0`, run it with
   `gh workflow run rotguard.yml` and check its log shows `Download rotguard 0.1.0 (x86_64)`.
   That confirms the action reads the tag from its folder name and that `if:` works on composite
   steps. If the log shows a build instead, the ref detection failed: the run is slower, not
   wrong, but fix `action.yml`.
4. After the second release, check `git ls-remote --tags origin v0` points at it. That confirms
   `v0` can still be force-pushed with immutable releases on. If not, make `v0` a branch that
   each release fast-forwards; sites don't change.
