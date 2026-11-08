# Plan: Trustworthy on foreign sites
Design: docs/design/foreign-sites.md
Risk paths: action.yml — T7 adds the `path` input and passes it to the run step's rotguard call.
Diff estimate: 250

### T1: Acceptance tests
- Files: tests/acceptance/foreign_sites.rs, tests/acceptance/lookup_order.rs, tests/acceptance/action_path.rs, tests/acceptance/main.rs, tests/acceptance/support.rs
- Interfaces: `support.rs`'s `Req` gains `pub query: Option<String>` (the request URL's query
  string), filled in `Log::push`; `path` stays exactly as it is, since `hits()` compares it.
  `foreign_sites.rs` holds S1 to S9 with `World`/`Routes`/`page` fixtures (unmatched paths,
  including `/`, answer 404 unless a test mounts `/`); assert verdict strings through
  `out.entry(url).verdict`, request counts through `w.log.hits("/")`, and "no archive request"
  through `w.alog.all().is_empty()`. `lookup_order.rs` holds S10: 1 dead link (404) at a path
  that sorts after both suspects (dead `/z/gone`, suspects `/a/1` and `/b/2`, each redirecting
  to root, so only a dead-first order passes) on pages with 3 different dates, a `Cdx` mock
  with captures for all 3, and the assertion that every `w.alog` request whose `url` query parameter is the
  dead URL comes before the first one for either suspect URL. `action_path.rs` holds S11 to S13
  and its own copy of the run-step helper (do not edit `action.rs`): S11 builds a temp
  workspace with the site under `blog/` (a fresh `World` site copied or symlinked there), runs
  the `id: run` script with `current_dir` = workspace and `ROTGUARD_PATH=blog`, and checks
  `blog/post.md` holds the redirect target and `exit-code=0`. S12 parses `action.yml` and
  checks: `inputs` has exactly `path`, with `default: "."` and `required: false`; the `id: run`
  step's `env.ROTGUARD_PATH` is `${{ inputs.path }}`; its script contains `ROTGUARD_PATH` after
  a ` -- ` (a regex, not an exact string); no run script contains `${{`. S13
  parses every `yaml` block of `docs/github-actions.md`.
- Test: one test per scenario, named `s<N>_…` after its S-ID, each with its own
  `#[ignore = "pending: S<N>"]`, S1 to S13
- Covers: S1, S2, S3, S4, S5, S6, S7, S8, S9, S10, S11, S12, S13
- Commit: Add acceptance tests for foreign sites

### T2: Probe redirected to the link
- Files: src/check.rs
- Interfaces: Produces `enum Soft404 { NoEvidence, Suspect(String) }` as the return type of
  `Probes::reason`; `check_one` maps `Suspect(r)` to
  `Verdict::Suspect { chain: fetched.chain.clone(), reason: r }` and `NoEvidence` to the verdict
  it had. T4 adds the `CantTell` variant (an unbuilt variant would fail clippy's `dead_code`).
  The not-found title check moves into a helper that `NoEvidence` paths call; a 404/410 probe still returns
  before it and a failed probe still falls through to it. Step 1a: probe final URL equals the
  link's final URL and the link chain has length 1 → `NoEvidence`; step 1b (chain longer) →
  `Suspect("probe: same final URL")`.
- Test: a row in `soft404_probe_verdicts` for a probe that redirects to the unredirected link
  (Ok); the existing "same final url" row still passes; un-pends S1, S2
- Covers: R1, R11, S1, S2
- Commit: Trust a probe redirected to the link

### T3: Same title, different text
- Files: src/check.rs
- Interfaces: Consumes `Soft404`. Step 2: titles match, body Jaccard < 0.5 and both shingle sets
  have at least 20 entries → `NoEvidence`. Constants `MIN_SHINGLES: usize = 20` and
  `DIFFERENT_BODY: f64 = 0.5` beside `SIMILAR_BODY`.
- Test: unit rows for 19 vs 20 shingles and Jaccard just under and at 0.5; un-pends S3
- Covers: R2, S3
- Commit: Ignore a shared title over different text

### T4: Pages that need JavaScript
- Files: src/check.rs
- Interfaces: Adds `Soft404::CantTell(String)`, which `check_one` maps to
  `Verdict::Unverifiable { reason }`. Step 3, after step 2 and only when a title or body match
  remains: the link's visible text (as `shingles` sees it, whitespace collapsed, lowercased)
  contains one of `enable javascript`, `javascript is required`, `requires javascript`,
  `javascript is disabled` → `CantTell("probe: page needs JavaScript")`. Before returning
  `CantTell`, the not-found title helper runs; a match returns
  `Suspect("title says \"<phrase>\"")` instead (R6).
- Test: unit test for the phrase matcher (case, extra whitespace, inside `<noscript>`, not inside
  `<script>`); un-pends S4
- Covers: R3, S4
- Commit: Report pages that need JavaScript

### T5: Root page as control
- Files: src/check.rs
- Interfaces: `Probes` gains `roots: StdMutex<HashMap<String, Arc<OnceCell<Arc<RootPage>>>>>`
  keyed by `url.origin().ascii_serialization()`; `enum RootPage { Usable { title: String,
  shingles: HashSet<String> }, Unusable }`, fetched once per origin through `Fetcher::fetch` for
  `<origin>/`, `Usable` only when the fetch ends in 2xx. Step 4, only after steps 1 to 3: when the
  root is usable and matches the probe on every signal the link matched (title equal; body
  `jaccard(root, probe) >= 0.5`) → `CantTell("probe: every URL gets the same page")` through the
  same R6 title helper; otherwise `Suspect("probe: same title")`, else
  `Suspect("probe: similar body")`.
- Test: un-pends S5, S6, S7, S8, S9; every existing soft-404, directory-probe, blocked and
  root-redirect test passes unchanged
- Covers: R4, R5, R6, R7, S5, S6, S7, S8, S9
- Commit: Compare probe matches with the site root

### T6: Dead lookups first
- Files: src/lib.rs
- Interfaces: `check_pages` builds the lookups as `Vec<(Url, NaiveDate)>`: dead keys sorted by
  URL then date, then suspect keys sorted the same way, a key in both kept once in the dead part.
  `look_up_snapshots` takes that `Vec` and awaits each `archive.snapshot(&url, date, deadline)`
  in order; its return map, `on_done` calls, deadline and `NoFix` reasons are unchanged.
- Test: un-pends S10; archive.rs s25 to s30 and s44 pass unchanged
- Covers: R8, S10
- Commit: Look up dead links before suspects

### T7: Action path input
- Files: action.yml, docs/github-actions.md, tests/acceptance/action.rs
- Interfaces: `action.yml` gains `inputs.path` (`description`: "Site directory with
  rotguard.toml, relative to the workspace root.", `required: false`, `default: "."`). The `run`
  step gets `env: ROTGUARD_PATH: ${{ inputs.path }}` and runs
  `"$GITHUB_ACTION_PATH/target/release/rotguard" --apply --format markdown -- "${ROTGUARD_PATH:-.}"`,
  so the locked run-step harnesses in `tests/acceptance/action.rs` and
  `tests/acceptance/markdown.rs`, which set no `ROTGUARD_PATH`, still run on the site dir.
  `docs/github-actions.md` gets a "Site in a subfolder" section after the first yaml block with a
  workflow whose `rdiachenko/rotguard@v0` step has `with: path: web/site`; the first block stays
  as it is. This changes an earlier design's locked test: `action_s4` in
  `tests/acceptance/action.rs` asserts `action.yml` has no inputs; replace that 1 assertion with
  one that the only input is `path`, default `.`, not required (open with `amend`).
- Test: action.rs s1 to s5 pass (s4 amended); un-pends S11, S12, S13
- Covers: R9, R10, S11, S12, S13
- Commit: Add the action's path input

### T8: Docs
- Files: docs/architecture.md, docs/config.md, README.md, CHANGELOG.md
- Interfaces: read the branch diff against `Base:` in `.whittle/run/progress.md`.
  `docs/architecture.md`: the "Soft-404 and bot-wall probes" paragraph (the 3 comparisons, the
  404-probe skip of the title check, checks 1 to 4 and the root control, the 2 UNVERIFIABLE
  reasons), the Check flow step 4 (dead lookups first), the action's run step and the
  `with: path` input, the "Soft-404 judgement" hard part, and a Decisions line pointing at
  `docs/design/foreign-sites.md` D1, D2. `docs/config.md` `[hosts."<name>"]`: soft-404 detection
  may also GET the host's root once and can report UNVERIFIABLE. README class table: the
  `UNVERIFIABLE` row adds "a host that gives every URL the same page, a page that needs
  JavaScript". CHANGELOG `## [Unreleased]`: 1 bullet each for the soft-404 change, dead-first
  lookups and the action's `path` input. No version bump.
- Test: none
- Covers: none
- Review: skip
- Commit: Update docs for foreign sites

## Verification contract
```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'
! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'
scripts/privacy-check.sh --all
```
