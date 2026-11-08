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
   Set `v=` in the README's install snippet to X.Y.Z too.
2. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [X.Y.Z] - <date>` and add an empty
   `## [Unreleased]` above it.
3. Run the checks in `CONTRIBUTING.md`.
4. Commit as `Release X.Y.Z`, push `main` and wait for CI to pass.
5. `git tag vX.Y.Z && git push origin vX.Y.Z`
6. `gh run watch` until the release run ends.
7. `gh release view vX.Y.Z` shows 3 assets: the 2 binaries and `SHA256SUMS`. Check that
   `git ls-remote --tags origin v<major>` shows the new commit.
8. Ask the user to list the release on the GitHub Marketplace: on the release page, Edit, tick
   "Publish this action to the GitHub Marketplace", Update release. Publishing with `gh` can't
   tick it, and only the repo owner can.

A failure before publishing leaves no release and `v<major>` where it was. Delete the tag with
`git push --delete origin vX.Y.Z && git tag -d vX.Y.Z`, fix the cause, and tag again. A published
release is final (immutable releases): fix forward with the next patch version. A run that fails
after publishing leaves the release in place; move `v<major>` by hand with
`git tag -f v<major> <sha> && git push -f origin v<major>`.
