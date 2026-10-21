# Plan: A redirect through the root is suspect only when the link ends there
Design: docs/design/root-redirect.md
Risk paths: none
Diff estimate: 14

### T1: Acceptance tests
- Files: tests/acceptance/root_redirect.rs (new), tests/acceptance/main.rs
- Interfaces: Consumes the CLI through `tests/acceptance/support.rs` (`World`, `Routes::fixed`,
  `redirect`, `page`, `w.site.write`, `w.site.read`, `w.run`, `w.url`, `Out::entry`,
  `Out::entries`, `Out::assert_code`).
- Test: one test per scenario S1–S4, named `root_redirect_s<n>_…`. Mock routes match the path
  only, so `fixed("/", …)` serves `M/?p=123`; S1's first redirect points at `w.url("/?p=123")`.
  S1 compares the trimmed detail with `fix: M/2020/post/` (as S12 in check.rs does), checks no
  entry's verdict starts with `SUSPECT`, and checks exit code 1. S2 runs with `--apply` and checks
  `post.md` still holds its original text. Only S1 is marked `#[ignore = "pending: S1"]`: today it
  is `SUSPECT redirect to root`. S2, S3 and S4 describe behaviour that stays as today (R3, R4,
  and R1's final root), so they pass already and carry no pending marker; they guard T2 against
  breaking it. The red run is S1's alone. `main.rs` is locked: open it with
  `whittle-gates amend` and add `mod root_redirect;` only.
- Covers: S1, S2, S3, S4
- Commit: Add acceptance tests for root redirects

### T2: Judge a 2xx chain by where it ends
- Files: src/check.rs
- Interfaces: `classify(outcome, host) -> Verdict` keeps its signature. The failed-chain arm keeps
  `redirects_to_root` (any hop after the first is `/`). The `_ if redirects_to_root(chain, host)`
  arm in `match last.status` stops matching 2xx statuses and keeps the any-hop test for the rest.
  In the `200..=299` arm, with `host.soft404` on and the first hop's path not `/`, a last hop
  whose path is `/`, or (when `leading > 0`) `chain[leading]` whose path is `/`, gives
  `Suspect { chain, reason: "redirect to root" }` before the Permanent, Temporary and Ok branches.
  Reword the doc comments on `redirects_to_root` and `classify`, which say "visits the root".
- Test: add rows to `redirects_to_the_root_classify_by_soft404_and_final_status`
  (src/check.rs:1273), with a small helper for a hop at any path:
  `/archives/123` 301 → `/?p=123` 301 → `/2020/post/` 200 is Permanent with fix `/2020/post/`;
  `/a` 301 → `/` 302 → `/b` 200 is Suspect; `/a` 302 → `/` 301 → `/b` 200 is Temporary (use
  `Expect::Is(Verdict::Temporary { chain, final_url })` built from that row's hops, with
  `final_url` the `/b` hop's URL); `/a` 301 → `/` 301 → `/x` 400 is Suspect. The 13
  existing rows don't change. Un-pends S1.
- Covers: R1, R2, R3, R4, S1
- Commit: Judge a 2xx chain by where it ends

### T3: Docs
- Files: docs/architecture.md
- Interfaces: none
- Test: none. Read the branch's diff against `Base:` in .whittle/run/progress.md.
  - docs/architecture.md:80-81 (soft-404 paragraph): "when the page redirected a non-root URL to
    the site root" becomes: when a non-root link ends at the site root, or its permanent fix would
    point there; a chain that never reaches a page is suspect if any hop is the root.
  - README.md:78 ("redirected to the site's home page") and docs/config.md:104 stay true; leave
    them.
- Covers: none
- Review: skip
- Commit: Update docs for root redirects

## Verification contract
```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'
! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'
scripts/privacy-check.sh --all
```
