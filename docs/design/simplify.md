---
status: approved
---
# Simplify rotguard to what the site needs

## Problem
rotguard can rewrite fixes into five kinds of link position, but the owner's site uses two. Its
38 content files all have TOML (`+++`) front matter (5 external URLs, keys `source` and `url`)
and 87 external body links, every one a markdown inline link `[text](url)`. It has no YAML
front matter, no raw-HTML `href`/`src` and no bare URLs. Making `--apply` safe for YAML took a
hand-written YAML lexer (~160 lines) and three review rounds of wrong-field fixes, and every
writable position adds edge cases the tests must pin. Narrowing `--apply` to markdown links and
TOML front matter removes that code and its tests. Every link stays checked and reported, and
nothing changes on the site.

## Scope
In:
- `--apply` writes only markdown link destinations and TOML front-matter strings, ~-200 lines
  (YAML lexer and helpers, `Quote::Bare`, the YAML and Html/MdText `writable` arms). The site
  needs it: 87 markdown links and 5 TOML URLs keep their fixes.
- YAML front-matter, raw-HTML and bare-URL links are still checked and reported, but their fixes
  are "manual fix". About +10 lines to report a YAML link at its own line without the lexer.
  Nothing on the site uses these forms.
- Test trim, ~-400 unit-test lines: delete tests of removed code, fold per-case tests into
  table-driven ones, and delete unit tests whose every assertion an acceptance test already makes.
- `docs/design/link-health.md` is updated where it describes what `--apply` writes (R23, the
  Extract and Data model notes). Docs only.

- Backlog lines this settles, deleted by the plan: the `[P2]` YAML-lexer-forms line, the `[P2]`
  sentinel speed/alias line (YAML aliases no longer reach it; TOML on the site has 5 URLs), and the
  `[P2]` lone trailing `\` on bare URLs. The `[idea]` on R23 rules for MdText and Html is settled
  here (both are manual fixes) and leaves the backlog with this design.

Out:
- Changing what is extracted or checked: classification, probes, the archive, the report wording,
  config and exit codes stay as they are.
- Removing YAML parsing (`serde_norway`), Html or bare-URL extraction: those links stay checked.
- The other backlog lines (R10, R11, R19 and DNS ideas; test-harness and config P2s).
- Running rotguard on the site (it has no `rotguard.toml` or CI step yet): see the backlog idea.

Size: ~-165 non-test lines net (about 175 removed, 10 added) and ~-400 unit-test lines;
acceptance tests grow by the new scenarios below (~80 lines). The plan's `Diff estimate:` carries
this target.

## Requirements
- R1: WHEN `--apply` is given THE SYSTEM SHALL write fixes only for links in markdown link destinations (`Ctx::MdDest`: inline, reference definition, autolink) and in TOML front-matter strings, under the rules of link-health R21–R23.
- R2: IF a fix belongs to a link in YAML front matter, a raw-HTML `href`/`src`, or a bare URL in text THEN THE SYSTEM SHALL leave the file's bytes for it unchanged and report the entry's detail line as `manual fix: not verbatim`.
- R3: THE SYSTEM SHALL extract, check, classify and report YAML front-matter, raw-HTML and bare-URL links as before, each entry naming the line where the URL is written.
- ~~R4: THE SYSTEM SHALL keep every link-health acceptance test (S1–S45) passing without editing its assertions.~~ Retired: it checks the branch, not the product; the build locks the existing acceptance tests and the plan's Diff estimate carries the size target.
- ~~R5: THE SYSTEM SHALL have at least 150 fewer non-test lines in `src/` and at least 300 fewer lines in `#[cfg(test)]` modules than at the base commit of the branch that builds this design.~~ Retired: it checks the branch, not the product; the build locks the existing acceptance tests and the plan's Diff estimate carries the size target.

## Acceptance scenarios
- S1 [R1, R2]: Given `post.md` with YAML front matter `source: "M/old"` and a body line `[x](M/old2)`, both answering 301 → `M/new` → 200, when rotguard runs with `--apply`, then the front-matter line is byte-for-byte unchanged, the body line reads `[x](M/new)`, and the front-matter entry's detail line is `manual fix: not verbatim`.
- S2 [R2]: Given a body line `<a href="M/old">x</a>` where `M/old` → 301 → `M/new` → 200, when rotguard runs with `--apply`, then `post.md` is unchanged and the entry's detail line is `manual fix: not verbatim`.
- S3 [R2]: Given a body line `see M/old here` where `M/old` → 301 → `M/new` → 200, when rotguard runs with `--apply`, then `post.md` is unchanged and the entry's detail line is `manual fix: not verbatim`.
- S4 [R1]: Given TOML front matter `source = "M/old"` and `url = 'M/old2'`, both answering 301 → `M/new` → 200, when rotguard runs with `--apply`, then the lines read `source = "M/new"` and `url = 'M/new'` and no other byte of the file changes.
- S5 [R3]: Given `post.md` with YAML front matter whose third line is `  link: "M/gone"` (nested under a key) and `M/gone` answering 404 twice, when rotguard runs, then the report has an entry headed `post.md:3  DEAD 404  M/gone`.
- S8 [R3]: Given YAML front matter whose lines 2 and 3 are `a: "M/gone"` and `b: "M/gone"`, and `M/gone` answering 404 twice, when rotguard runs, then the report has two entries for `M/gone`, headed `post.md:2  DEAD 404  M/gone` and `post.md:3  DEAD 404  M/gone`.
- ~~S6 [R4]: Given the branch that builds this design, when its acceptance suite runs, then every S1–S45 test of link-health passes and `git diff <base> -- tests/acceptance` changes no existing assertion (it may only add the new tests and their module line).~~ Retired: it checks the branch, not the product; the build locks the existing acceptance tests and the plan's Diff estimate carries the size target.
- ~~S7 [R5]: Given the base commit and the branch head, when non-test lines in `src/` and lines inside `#[cfg(test)]` modules are counted, then the head has at least 150 fewer of the first and 300 fewer of the second.~~ Retired: it checks the branch, not the product; the build locks the existing acceptance tests and the plan's Diff estimate carries the size target.

## Design
One mechanism: `Link::verbatim` already means "this span is exactly the source bytes of the URL,
so `--apply` may write it" (link-health, Data model). Extraction sets `verbatim: false` for every
position `--apply` must not write, and `apply::writable` turns `verbatim: false` into
`Unwritable::NotVerbatim` ("manual fix: not verbatim") as it does today. Nothing else in `check`,
`archive`, `report` or `lib` changes.

```text
Context: rotguard (unchanged)
                    ┌─────────────┐
                    │ Site owner  │
                    └──────┬──────┘
         runs, reads report│ ▲ exit code
                    ┌──────▼──────┐  fetch URL (HTTPS)   ┌──────────────────┐
  site sources ────►│  rotguard   ├─────────────────────►│ linked websites  │
  (markdown; only   │  [Rust CLI] │                      └──────────────────┘
   --apply writes)  └──────┬──────┘  closest snapshot    ┌──────────────────┐
                           └────────────────────────────►│ Wayback Machine  │
                                     (HTTPS, JSON API)   └──────────────────┘
```

```text
Flow: which link positions --apply may write
  extract ──1. Link{url, span, ctx, verbatim}──► check / archive ──2. Finding{fix}──► apply
    │ MdDest, verbatim when source bytes == URL ............. verbatim: true|false (as today)
    │ TOML front-matter string, confirmed by the sentinel re-parse . verbatim: true|false
    │ YAML front matter, Html href/src, bare URL (MdText) .......... verbatim: false, always
  apply ──3. writable(fix, link)──► verbatim && quotable ? write span : "manual fix: <reason>"
```

Changes by file:
- `src/extract.rs`:
  - YAML front matter: `front_matter_links` no longer lexes YAML. Each URL value `parse_values`
    returns becomes a link with `verbatim: false`, `ctx: FrontMatter { quote: Double }` (the
    placeholder the leftover-values branch already uses), and a span at the first occurrence
    of the URL's text in the block at or after a cursor. The cursor starts at the block start
    and moves to the end of each span found, so repeated values get their own lines in
    document order (`parse_values` returns YAML values in mapping and sequence order). A
    value whose text doesn't occur after the cursor (written with escapes) gets the
    whole-block span, as today, and leaves the cursor where it was.
  - Removed: `lex_yaml`, `enum Line`, `is_block_indicator`, `lex_yaml_line`, the YAML arms of
    `decode`, the `doubled` parameter of `closing_quote` if only YAML used it, and `Quote::Bare`.
  - TOML is unchanged: `lex_toml`, pairing, the decoded path and the sentinel `Verify` check stay.
  - Html `href`/`src` and bare URLs (`scan_bare`) are pushed with `verbatim: false`. Their spans
    and URL decoding stay as they are, so lines and checked URLs are unchanged.
- `src/apply.rs`: `writable` loses the `Quote::Bare` arm. The `Ctx::MdText | Ctx::Html` arm can't
  be reached with `verbatim: true` any more, so it returns `Unwritable::NotVerbatim`.
- Tests:
  - Delete unit tests that only cover removed code: the YAML lexer tests and the phantom-token
    tests. Rewrite them as whole-value checks where the behaviour stays (YAML URLs, date and
    line still extracted).
  - Fold per-case tests into one table each: the check.rs redirect-to-root group, the config.rs
    rejected-value group, the extract.rs TOML and bare-URL groups, the report.rs per-class
    prints.
  - Delete a unit test only when an acceptance test makes every one of its assertions. Keep the
    span and `ctx` assertions that no acceptance test makes.
  - Acceptance: S1–S5 and S8 above are new tests in a new acceptance module. Adding its `mod` line to
    the locked `tests/acceptance/main.rs` is an amendment that only adds tests.
- `docs/design/link-health.md`:
  - R23's quote rules say "TOML" where they now say "TOML or YAML".
  - The Extract note on YAML plain-scalar spans and the `Quote::Bare` data-model note are
    replaced by a pointer to this design.

## Decisions
### D1: One mechanism, `verbatim: false`, for every position `--apply` won't write
Options:
- a new `ManualReason` such as "not writable here";
- marking the links `verbatim: false` at extraction.

Chosen: `verbatim: false`.

Why: it reuses the existing refusal path and its report text, and needs no new type or wording.

Cost if wrong: the reason reads "not verbatim" for a position that is simply never written. A
clearer reason can be added later as one enum variant.

### D2: Keep the sentinel re-parse check for TOML
Options: delete `Verify` (29 lines) and trust the TOML lexer, or keep it.

Chosen: keep it.

Why: it guards the write path against a lexer misread, such as a quoted dotted key
`links."https://a.test/p".x = 1` next to a real value. At the site's 5 front-matter URLs its
re-parse cost is nothing.

Cost if wrong: 29 lines kept.

### D3: Bare URLs are never rewritten (the owner's choice)
Options: keep `Ctx::MdText` writable, or make it a manual fix.

Chosen: manual fix.

Why: the site has none, and it settles the open bare-URL backlog lines (the lone trailing `\`
and the R23 rules for plain text).

Cost if wrong: a site that gains bare URLs edits them by hand.

### D4: A YAML link is reported at the line where its URL text first appears
Options: the block's first line for every YAML link, or a text search.

Chosen: search for the URL text, in document order, falling back to the whole block.

Why: R3 keeps report lines accurate, for about 10 lines of code (S5, S8).

Cost if wrong: a URL that also appears inside an earlier non-URL scalar is reported a few
lines early. The span is never written, so nothing else is affected.

## Risk paths
Unchanged from `.whittle/gates.toml`: `.github/**`, `.githooks/**`, `scripts/**`, `Cargo.toml`,
`Cargo.lock`, `**/*secret*`, `.env*`, `.whittle/gates.toml`, `src/apply.rs`. This change touches
`src/apply.rs` (one `writable` arm), so the plan must list it.

## Fitness rules
Unchanged:
- Only `src/apply.rs` writes files: `! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'`.
- Tests never hit the real network (the tests URL grep).
- `scripts/privacy-check.sh --all`.

## Review changes
- Retired R4, R5, S6 and S7 at the owner's request: they checked the branch, not the product, and a git-based test breaks after the merge. The build locks the existing acceptance tests; Size and the plan's Diff estimate keep the size target.
- Round 1: the YAML span search runs from a cursor that moves to each found span's end and is left alone by a whole-block fallback, so repeated URLs are reported at their own lines; S8 covers it (Design, D4).

## Open for you
none
