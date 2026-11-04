# Contributing

Bug reports, misclassified links and pull requests are welcome. For anything bigger than a fix,
open an issue first so we can agree on the behaviour before you write the code.

## Report a bug

Use the bug report form. The most useful report has the command, your `rotguard.toml` and the
report entries for the links involved. For a link rotguard got wrong, say what you see when you
open it in a browser; `--format json` shows the whole redirect chain rotguard followed.

## Set up

You need Rust 1.88 or newer (`rustup toolchain install stable`).

```sh
git clone https://github.com/rdiachenko/rotguard
cd rotguard
git config core.hooksPath .githooks   # privacy check on every commit
cargo test
```

The hooks run `scripts/privacy-check.sh`, which blocks home paths, temp paths, tokens, keys and
email addresses other than yours from entering the history. If it flags a false positive, fix the
pattern in the script in the same pull request.

## Check your change

CI runs these, and a pull request needs all of them green:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
scripts/privacy-check.sh --all
```

The code keeps 3 rules, checked by the commands in `.whittle/gates.toml`:

- Only `src/apply.rs` writes files.
- Tests never touch the real network. They serve pages with `wiremock` on `127.0.0.1`, and any
  other host in a test ends in `.test`, `.invalid` or `.example`.
- No private data in tracked files.

## Find your way around

[docs/architecture.md](docs/architecture.md) maps the modules and the main flows. Unit tests sit
next to the code in `src/`; end-to-end tests that run the binary against mock sites are in
`tests/acceptance/`, one module per feature.

Each feature started as a design in `docs/design/` with numbered requirements (`R1`) and
acceptance scenarios (`S1`), then a plan of test-first tasks in `docs/plans/done/`. Acceptance
test names and commit messages (`Covers: R1, S1`) point back to them, so the design is the place
to look for why the code behaves as it does. You don't need to write a design for a fix; for a new
feature, an issue describing the problem is enough. `.whittle/` configures the workflow tool that
produced these documents; you can ignore it.

## Write commits

A commit does one thing. The subject is a short imperative sentence ("Probe a sibling for a path
ending in /"), and the body, when there is one, says why. User-facing changes also update
`README.md`, `docs/config.md` or `docs/github-actions.md`, and `CHANGELOG.md`.

## License

Unless you say otherwise, any contribution you submit for inclusion in rotguard is dual-licensed
under MIT or Apache-2.0, as in the README, without any additional terms or conditions.
