---
status: approved
mode: epic
---
# Trustworthy on foreign sites

## Overview

### Summary
rotguard learns to tell a soft-404 from a host that answers every URL alike, so a run on a site
unlike your blog stops drowning its real findings in false SUSPECTs. On the affilytics blog, all
but 3 of its 102 suspects become OK or UNVERIFIABLE with a reason, and 1 of those 3 is real.
Dead-link lookups run before suspect lookups, so the fixes `--apply` can write get the archive's
time first. The GitHub Action gains a `path` input for a site in a monorepo subfolder.

Size: ~430 lines; 0 questions wait under Open for you.

### Problem
A read-only run on the affilytics blog (100 pages, 241 links) reported 102 SUSPECT links, and a
look at each showed only 1 of them is a real soft-404. Most carry `probe: same title`: Amazon
Associates help sites, reddit, partners.target.com and a few client-rendered apps give a made-up
URL the same `<title>` as a real page. 5 carry `probe: same final URL`: webkit.org, The Verge,
Statista and PCMag route by an ID and redirect any slug to the real article, so the probe lands
on the link itself.

Every SUSPECT also gets a Wayback lookup at 1 per 4 s. The check phase took 112 s, then 246
lookups queued behind a 300 s deadline, so 14 of the 16 real dead-link findings (13 of them
www.ftc.gov 404s) ended `manual fix: deadline` and `--apply` had nothing to write.

Turning `soft404` off per host would take about 19 entries, grow with each post and blind those
hosts to real soft-404s. And the site lives in `web/affilytics-website/blog` of a monorepo, which
the action can't reach: it always runs rotguard in the workspace root.

### What you can do with it
1. Run `rotguard` on the affilytics copy: the summary drops from `suspect 102` to about
   `suspect 3`. Reddit threads read `UNVERIFIABLE probe: every URL gets the same page`, the
   affilytics app's `/signup` reads `UNVERIFIABLE probe: page needs JavaScript`, and most Amazon
   help pages and the webkit.org article are OK.
2. Dead links get their snapshots first: on the affilytics run all 15 dead lookups fit in the
   deadline, so the ftc.gov 404s get `fix: https://web.archive.org/web/…` instead of
   `manual fix: deadline`. A slow archive day can still cut some off.
3. Check a site in a monorepo from its weekly workflow:
   ```yaml
   - id: rotguard
     uses: rdiachenko/rotguard@v0
     with:
       path: web/affilytics-website/blog
   ```

### Not in this version
- JavaScript rendering. A page that needs it is reported UNVERIFIABLE, never judged.
- support.google.com threads and advertising.amazon.com partner pages stay SUSPECT. They ignore
  the slug and serve the article at the made-up URL without a redirect, which looks exactly like
  a soft-404 to a probe. Parked as an `[idea]`: a second probe that also replaces the ID.
- 1 CDX query per URL shared across page dates. Dead-first ordering fits the affilytics lookups
  in the deadline; parked as an `[idea]`.
- A config key for the new checks or their thresholds. `soft404 = false` stays the per-host
  escape hatch, for example for reddit's 50 UNVERIFIABLE threads.
- The 0.2.0 release (version bump, tag) and wiring rotguard into the affilytics repo: another
  session does both after the merge.

### How it works
The change sits inside the soft-404 judgement in `check.rs` (`Probes::reason`), the snapshot
lookup loop in `lib.rs` and the action's run step. Nothing new talks to the outside world:
rotguard may now also GET a host's root page, through the same polite `Fetcher`.

```text
Context: rotguard (changes marked *)

                  ┌──────────────┐
                  │ Site owner   │
                  └──────┬───────┘
  rotguard [site_dir]    │ ▲ report on stdout, exit code 0 / 1 / 2
                  ┌──────▼─┴─────────────┐  reads rotguard.toml   ┌──────────────────────┐
  action.yml ────►│ rotguard             ├───────────────────────►│ Site directory       │
  *with: path     │ [Rust, one binary]   │  and content           │ (*any subfolder)     │
                  └──────┬──────────┬────┘                        └──────────────────────┘
  GET link, probe and    │          │ *dead lookups first, then suspect
  *the root (HTTPS)      ▼          ▼
            ┌──────────────────┐ ┌──────────────────────────┐
            │ Linked websites  │ │ Wayback Machine CDX API  │
            └──────────────────┘ └──────────────────────────┘
```

#### Separating a soft-404 from a host that answers alike
Today a 2xx link is SUSPECT when a made-up URL beside it (the probe) ends at the same URL, has
the same title or has a similar body. That reads "this page is the host's not-found page". But a
match also happens when the host answers every URL alike: an app shell, a bot-check page, one
site-wide title, or a redirect from any slug to the real article. The new checks look for that
second explanation before saying SUSPECT, and only ever turn a SUSPECT into OK or UNVERIFIABLE.

The checks run in this order, and the first that applies decides:
1. The probe was redirected to the link itself, and the link answered with no redirect: the
   host recognised the ID in the path, so the probe is no evidence. A link that was itself
   redirected to where the probe ends, as thinkwithgoogle.com now sends every URL to 1 landing
   page, stays SUSPECT.
2. Only the titles match, and both pages have enough text to compare but share less than half
   of it: they are 2 different pages under 1 site-wide title, as on the Amazon help sites.
3. The link page asks for JavaScript ("enable JavaScript"): rotguard can't see the real page, so
   it says UNVERIFIABLE.
4. rotguard fetches the host's root once and compares it with the probe. When the root matches
   the probe on everything the link matched (same title, mostly the same text), every URL gets
   the same page and the result is UNVERIFIABLE. Otherwise the match stands and the link is
   SUSPECT, as today.

The root works as a control because it's a page that exists by definition. A real not-found
page looks unlike the home page; an app shell or bot-check page looks like it. When the root
can't be fetched (it fails or answers 404), there is no control and the old rule decides. That
keeps every existing soft-404 test, whose fixtures serve no root, passing unchanged.

A not-found phrase in the link's title still wins: a page titled "Page not found" is SUSPECT
even when checks 3 or 4 would say UNVERIFIABLE.

```text
Flow: judge a 2xx link whose probe answered 2xx
  link ─1. probe ends at link, link not redirected ─► no evidence ─► title phrase ─► OK | SUSPECT
    │ 1b. link redirected, probe ends at the link's final URL ─► SUSPECT probe: same final URL
    │ 2. title only, texts ≥ 20 shingles, < 0.5 alike ─► no evidence ─► title phrase ─► OK | SUSPECT
    │ (no title or body match) ─────────────────────► title phrase check, as today
    │ 3. link text asks for JavaScript ─► title phrase? ─yes─► SUSPECT title says …
    │                                                   └no──► UNVERIFIABLE needs JavaScript
    └ 4. GET / (once per origin) ─┬ root fails or differs ─► SUSPECT probe: same title | body
                                  └ root matches probe ─► title phrase? ─► SUSPECT | UNVERIFIABLE
```

The checks can fail silently in 1 direction: a real soft-404 that shares a site-wide title with
the home page, or whose not-found page varies a lot per URL (a search page listing results for
the slug), is now OK or UNVERIFIABLE instead of SUSPECT. The thresholds come from the affilytics
hosts: the Amazon pages that differ share at most 0.29 of their text, the app shells match at
0.98 to 1.0, and the old tests' title-matching fixtures have under 20 shingles per page.

#### Dead lookups first
Snapshot lookups go to a single Wayback gate, 1 request per 4 s, in a random order today. They
now run in a fixed order: every dead link's lookups, then every suspect's. A dead link's
snapshot is a fix `--apply` writes; a suspect's is only a proposal, so it gets the time left
over. With the false suspects gone, the affilytics run needs about 20 lookups instead of 246.

#### The action's `path` input
`action.yml` gets 1 optional input, `path`, default `.`, the site directory relative to the
workspace root. The run step reads it from an environment variable, never from `${{ }}` inside
the script, and passes it as rotguard's site argument. The report's file paths stay relative to
that folder, and the create-pull-request step works as before because it commits from the
repository root. `docs/github-actions.md` gets a monorepo example.

### Decisions
#### D1: Use the host's root page as the control for a probe match
- Why: on the affilytics hosts, the root shares the probe's title and text exactly where the
  host answers every URL alike (reddit, partners.target.com, help.instagram.com), and differs
  where a real not-found page would. It costs at most 1 request per host, only when a probe
  matched, and the old tests keep passing because their fixtures serve no root.
- Rejected: a second made-up URL. Two made-up URLs always match each other, so it can't tell a
  shell from a not-found page.
- Rejected: a built-in list of hosts. Lists go stale, and probing over lists is an existing
  decision (`docs/design/link-health.md`, D4).
- Cost if wrong: the control lives in 1 function, `Probes::reason`; changing it is a day's work
  with its tests.

#### D2: "Can't tell" is UNVERIFIABLE, not OK and not BLOCKED
- Why: the link page, the probe and the root are the same page, so rotguard has no evidence
  either way. UNVERIFIABLE says so in the report and doesn't fail a run without `--strict`.
- Rejected: OK. It would claim a check that never happened; a deleted reddit thread looks the
  same.
- Rejected: BLOCKED. It is counted but not listed, which would hide 53 links, and it carries a
  status code where this case has a reason.
- Cost if wrong: about 50 reddit lines in each report. `[hosts."www.reddit.com"] soft404 = false`
  silences them today; switching the verdict later is a few lines in `check.rs` and its tests.

#### Smaller decisions
- D3: Dead lookups run strictly before suspect lookups, one after another, because the Wayback
  gate allows 1 request at a time anyway. Interleaving or a time share is a change to 1 loop.
- D4: The needs-JavaScript phrases are built in (`enable javascript`, `javascript is required`,
  `requires javascript`, `javascript is disabled`), matched without case on the link's visible
  text. A config key can come later if a site needs another phrase.
- D5: The input is named `path`, as in `actions/checkout`. Renaming it later is a breaking change.
- D6: "Enough text" is at least 20 three-word shingles per page, "different" is below 0.5 Jaccard
  similarity and "the root matches" is at or above 0.5. Each is 1 constant.

### Assumptions
- A1: No interview questions were asked. The idea states what the change does, who uses it, what
  done looks like and what is out.
- A2: UNVERIFIABLE beats OK for a host whose pages all look alike (D2). The idea prefers saying
  "can't tell" over a wrong SUSPECT and leaves the OK trade-off to the design; a confident OK on
  a page rotguard never saw is the same mistake in the other direction.
- A3: Fixing the false suspects plus dead-first ordering is enough for the deadline, with no
  shared CDX query. Measured: 112 s of checking, and CDX queries of 0.75 to 4.2 s behind the 4 s
  gate, so about 20 lookups (15 dead, the rest suspect) at 1 to 2 queries each fit the remaining
  188 s, dead links first. A slow archive day can still run past it (see Budgets).
- A4: The 2 support.google.com and advertising.amazon.com links stay SUSPECT. Telling them apart
  needs a second probe that replaces the ID, which a month-archive soft-404 (`/2020/05/<slug>`)
  would fool; parked as an idea.
- A5: The out-of-date parts of `docs/architecture.md` (it omits that a 404 probe skips the
  title-phrase check, and doesn't name the 3 probe comparisons) are fixed by the same build,
  since it rewrites that paragraph anyway.
- A6: Tests for these behaviours use local wiremock fixtures in `tests/acceptance/soft404.rs`
  and a new test file for the action input, never the live hosts.

### Scope
- Soft-404 checks 1 to 4 and the root control in `src/check.rs`: ~140 lines, for 99 of the 102
  suspects on the affilytics blog.
- Dead-first lookup order in `src/lib.rs`: ~25 lines, for the 15 dead-link lookups that missed
  the deadline.
- Docs: `docs/architecture.md`, `docs/config.md` (`soft404`), README class table,
  `docs/github-actions.md` monorepo section, CHANGELOG `[Unreleased]`: ~80 lines.
- `path` input in `action.yml`: ~6 lines, for the monorepo site.

Size: ~430 (250 estimated × 1.7, median of the last 5 cross-repo design builds)
Touches: `action.yml` — the new `path` input and the run step's site argument.

### Open for you
none

## Spec

### Requirements
- R1: WHEN a link's 2xx fetch had no redirect and its 2xx probe ended at the link's URL, THE
  SYSTEM SHALL not treat the probe as soft-404 evidence.
- R2: WHEN a link and its 2xx probe have the same non-empty title, each body has at least 20
  shingles, their Jaccard similarity is below 0.5 and R11 does not apply, THE SYSTEM SHALL not treat the probe as
  soft-404 evidence.
- R3: IF a 2xx probe matches the link by title or body, neither R1, R2 nor R11 applies, and
  the link's visible text contains a needs-JavaScript phrase, THEN THE SYSTEM SHALL mark the link `UNVERIFIABLE` with reason
  `probe: page needs JavaScript`.
- R4: IF a 2xx probe matches the link by title or body, neither R1, R2, R3 nor R11 applies,
  and the host's root page answered 2xx with the probe's
  title wherever the titles matched and a body at least 0.5 similar to the probe's wherever the
  bodies matched, THEN THE SYSTEM SHALL mark the link `UNVERIFIABLE` with reason
  `probe: every URL gets the same page`.
- R5: WHEN a 2xx probe matches the link and neither R1 to R4 applies, THE SYSTEM SHALL mark the
  link `SUSPECT` with the existing reason (`probe: same final URL`, `probe: same title` or
  `probe: similar body`).
- R6: WHERE R3 or R4 applies and the link's title contains a `not_found_phrases` entry, THE
  SYSTEM SHALL mark the link `SUSPECT` with reason `title says "<phrase>"` instead.
- R7: THE SYSTEM SHALL fetch a host's root page at most once per origin per run, and only when
  R4 is evaluated.
- R8: WHEN snapshot lookups are pending for both dead and suspect links, THE SYSTEM SHALL send
  every dead lookup's CDX requests before any suspect lookup's.
- R9: WHERE the action's `path` input is set, THE SYSTEM SHALL run rotguard on that directory,
  relative to the workspace root; unset, it SHALL run on the workspace root as before.
- R10: THE SYSTEM SHALL document the `path` input in `docs/github-actions.md` with a monorepo
  workflow example.
- R11: WHEN a link's fetch was redirected and its 2xx probe ended at the link's final URL, THE
  SYSTEM SHALL mark the link `SUSPECT probe: same final URL`, whatever the page text or the root
  says.

### Acceptance scenarios
- S1 [R1]: Given `/blog/15697/post/` answers 200 and `/blog/15697/<token>/` answers 302 to
  `/blog/15697/post/`, when rotguard runs, then the link is OK and the summary reads `suspect 0`.
- S2 [R1, R11]: Given `/docs/real`, `/docs/<token>` and `/` all answer 302 to `/docs/landing`,
  which answers 200 with a body reading `Please enable JavaScript`, when rotguard runs, then the
  link is `SUSPECT probe: same final URL`.
- S3 [R2, R3, R7]: Given `/help/topic/g1` and `/help/topic/<token>` share the title
  `Associates Central - Help`, `/help/topic/g1` has 400 distinct words plus the sentence
  `Please enable JavaScript for the full experience` and the probe 100 other words, when rotguard runs, then the link is OK and the server log has no request for `/`.
- S4 [R3]: Given `/signup` and `/<token>` serve the same page titled `App` whose body reads
  `You need to enable JavaScript to run this app.`, and `/` serves a different landing page,
  when rotguard runs, then the link is `UNVERIFIABLE probe: page needs JavaScript` and the exit
  code is 0.
- S5 [R4]: Given `/r/x/comments/1/post/`, its probe and `/` all serve the title `Reddit` with an
  empty body, when rotguard runs, then the link is `UNVERIFIABLE probe: every URL gets the same
  page`, the exit code is 0 and the archive mock received no request.
- S6 [R4]: Given `/faqs` and its probe serve the same title and the same 200-word body, and `/`
  serves that title with a body that shares about 85% of those shingles, when rotguard runs, then
  the link is `UNVERIFIABLE probe: every URL gets the same page`.
- S7 [R5]: Given `/blog/gone` and its probe serve the same title `Oops` and the same 200-word
  body, and `/` serves `Home` with unrelated text, when rotguard runs, then the link is
  `SUSPECT probe: same title`.
- S8 [R6]: Given `/z/a`, its probe and `/` all serve the page titled `Page not found` with an
  empty body, when rotguard runs, then the link is `SUSPECT title says "not found"`.
- S9 [R7]: Given `/a/x` and `/b/y` each match their own probe by title and body, and `/` matches
  both probes, when rotguard runs, then both links are UNVERIFIABLE and the server log holds
  exactly 1 request for `/`.
- S10 [R8]: Given 1 dead link and 2 suspect links whose pages carry 3 different dates, when
  rotguard runs, then in the archive mock's request log every CDX request for the dead URL comes
  before the first request for either suspect URL.
- S11 [R9]: Given a workspace whose site lives in `blog/` with a link that permanently redirects,
  when the action's run step runs from the workspace root with input `path` set to `blog`, then
  `blog/post.md` holds the redirect target and the `exit-code` output is 0.
- S12 [R9]: Given `action.yml`, when it is parsed, then it declares 1 input `path` with default
  `.` and `required: false`, the run step maps it into an environment variable, and no run
  script contains `${{`.
- S13 [R10]: Given `docs/github-actions.md`, when its yaml blocks are parsed, then one workflow's
  `rdiachenko/rotguard@v0` step has `with.path` set to a subfolder, and the first block is
  unchanged in shape.

### Details

**Where the checks go.** `Probes::reason` returns an enum instead of `Option<String>`:

```rust
enum Soft404 {
    /// No probe evidence; the not-found title check still runs (existing behaviour).
    NoEvidence,
    Suspect(String),
    /// Becomes `Verdict::Unverifiable { reason }` unless the title check matches.
    CantTell(String),
}
```

`check_one` maps `Suspect` to `Verdict::Suspect` with `fetched.chain`, as today, and `CantTell`
to `Verdict::Unverifiable`, which has no chain. The title-phrase check is a helper both
the `NoEvidence` path and the `CantTell` path call (R6). A probe that answered 404 or 410 still
returns before the phrase check, and a failed probe still falls through to it, unchanged.

**Decision order** for a non-root link whose fetch ended 2xx and whose probe answered 2xx:

| Step | Condition | Result |
|---|---|---|
| 1a | probe final URL == link final URL, link chain length 1 | `NoEvidence` |
| 1b | probe final URL == link final URL, link chain longer | `Suspect("probe: same final URL")` |
| – | titles differ (or empty) and Jaccard < 0.9 | `NoEvidence` (as today) |
| 2 | titles match, Jaccard < 0.5, both ≥ 20 shingles | `NoEvidence` |
| 3 | link visible text has a needs-JavaScript phrase | `CantTell("probe: page needs JavaScript")` |
| 4a | root usable; every matching signal also matches root→probe | `CantTell("probe: every URL gets the same page")` |
| 4b | otherwise | `Suspect("probe: same title")`, else `Suspect("probe: similar body")` |

"Root usable" means the root fetch ended in 2xx. Title signal matches root when the root's title
equals the probe's; body signal matches root when `jaccard(root, probe) >= 0.5`. Visible text is
what `shingles` already sees: `strip_hidden`, tags removed, entities decoded, lowercased, with
whitespace collapsed before the phrase match.

**Root cache.** `Probes` gains `roots: StdMutex<HashMap<String, Arc<OnceCell<Arc<RootPage>>>>>`
keyed by `url.origin().ascii_serialization()`. `RootPage` is `Usable { title, shingles }` or
`Unusable`. The fetch goes through `Fetcher::fetch` (redirects followed, per-host spacing, the
link's deadline) for `<origin>/`. It runs only in step 4.

**Lookup order.** `check_pages` builds the lookups as an ordered `Vec<(Url, NaiveDate)>`: dead
keys sorted by URL then date, then suspect keys sorted the same way, a key in both lists kept
once in the dead part. `look_up_snapshots` runs them one after another in that order; the result
map, progress callback, deadline and `NoFix` reasons are unchanged. The acceptance harness's
`Req` (`tests/acceptance/support.rs`) also records the request's query string, so S10 can tell
the CDX lookups apart by their `url` parameter.

**Action.** `inputs.path`: `description` "Site directory with rotguard.toml, relative to the
workspace root.", `required: false`, `default: "."`. The `run` step gets
`env: { ROTGUARD_PATH: "${{ inputs.path }}" }` and calls
`"$GITHUB_ACTION_PATH/target/release/rotguard" --apply --format markdown -- "$ROTGUARD_PATH"`.
The `--` keeps a folder whose name starts with `-` from being read as a flag.
A missing folder makes rotguard exit 2 (no `rotguard.toml`), which the workflow already reports
as a failed run. The test harness in `tests/acceptance/action.rs` sets `ROTGUARD_PATH` to the
input it simulates, `.` by default; `action_s4` changes from "no inputs" to S12's shape.

**Docs.** `docs/architecture.md`: the soft-404 paragraph (the 3 comparisons, the 404-probe skip,
checks 1 to 4, the root control), the check flow's lookup order, the action's run step and the
"Soft-404 judgement" hard part. `docs/config.md`: the `soft404` effect mentions the root request
and the 2 UNVERIFIABLE reasons. README: the `UNVERIFIABLE` row names "a host that gives every URL
the same page or needs JavaScript". `docs/github-actions.md`: a "Site in a subfolder" section
with the `with: path:` example after the first block. CHANGELOG `[Unreleased]`: 1 bullet each
for the soft-404 change, dead-first lookups and the `path` input.

- **Facts**:
  - probed: `curl`-like GETs with `User-Agent: rotguard/0.1.1` to 16 links, their probes and
    their roots (scratchpad `probe/p.py`). webkit.org, statista.com, uk.pcmag.com and
    theverge.com redirect the probe to the link itself; thinkwithgoogle.com sends link, probe
    and root to 1 landing page.
  - probed: Amazon Associates help pages share the probe's title but share 0.009 to 0.29 of its
    shingles (link 290 to 8841 shingles, probe 85 to 142); `/help/topic/t405` shares 0.82 and
    is a JavaScript-only shell.
  - probed: reddit serves link, probe and root the title `Reddit` with 0 shingles;
    partners.target.com serves all 3 identical (8 shingles); help.instagram.com's root shares
    0.85 with the probe.
  - probed: the affilytics app, sellercentral.amazon.co.uk and 2 Amazon help topics say "enable
    JavaScript" in their visible text.
  - probed: the rule simulated on all 103 suspect URLs of the affilytics report (`probe/sim.py`,
    before check 3): 53 UNVERIFIABLE, 41 OK, 8 SUSPECT, 1 fetch timeout. 5 of the 8 say
    "enable JavaScript", leaving 3: support.google.com, advertising.amazon.com and the true
    thinkwithgoogle.com.
  - probed: the current binary on the affilytics copy with `deadline_s = 1500` finished checking
    241 links in 112 s, then queued 246 snapshot lookups.
  - probed: 8 CDX queries (both sides) for 4 of the affilytics dead URLs, 4 s apart, took 0.75 to
    4.2 s each (median 1.4 s), all 200. So a query costs about max(4 s, its response time).
- **Invariants**:
  - The new checks only turn what the old rule calls SUSPECT into OK (`NoEvidence`) or
    UNVERIFIABLE; a link the old rule calls OK, PERMANENT or TEMPORARY keeps its verdict. Owner:
    `Probes::reason`.
  - Root and probe fetches go through `Fetcher` and its per-host gate, inside the link's
    deadline.
  - Only `src/apply.rs` writes files; UNVERIFIABLE links get no lookup and no fix.
- **Budgets**: affilytics, deadline 300 s: checking 112 s measured, plus at most 1 root request
  per host whose probe matched (about 15 hosts, spread across hosts, unmeasured). Lookups: about
  20 keys at 1 to 2 CDX queries of about 4 s each (measured, see Facts), 80 to 170 s; the 15 dead
  keys come first and take 60 to 130 s, inside the 188 s left. A slow archive day (retries,
  responses over 4 s) can still push suspect lookups, and in the worst case some dead ones, past
  the deadline; they then read `manual fix: deadline` as today. Fallback: a higher `deadline_s`
  in the site's config, or the parked shared-CDX-query idea.

### Risk paths
`action.yml` (new input); `src/apply.rs` is not touched. The rest of `.whittle/gates.toml`'s
risk paths are unchanged.

### Fitness rules
Unchanged: only `src/apply.rs` writes files; tests reach only local hosts; the privacy check.

### Review changes
- Round 1: added R11 (a redirected link whose probe ends at its final URL stays SUSPECT) and made
  R3 and R4 yield to it; S2 now also serves `/` and a JavaScript phrase on the landing page.
- Round 1: `Suspect` keeps the fetched chain; only `CantTell` drops it.
- Round 1: measured CDX latency, added it to Facts, and rewrote Budgets and A3 with the slow-day
  case and its fallback.
- Round 1: the acceptance harness records the query string so S10 can tell lookups apart.
- Round 1: corrected the shingle count of the old fixtures.
- Round 3: S3's link page also asks for JavaScript, so a build that runs check 3 before check 2
  fails it; the flow shows the title-phrase SUSPECT after checks 1 and 2; counts and "most Amazon
  help pages" corrected.
- Round 2: R2 yields to R11, R3 to R1, R2 and R11, and R4 to R1, R2, R3 and R11, matching the
  decision order.
- Round 2: the overview no longer promises that dead-link fixes always beat the deadline; it
  says they go first and fit on the affilytics run.
- Round 1: the run step passes `--` before the path (parked by the judge; fixed as it costs 2
  characters).
