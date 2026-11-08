---
status: approved
mode: epic
---
# Stop rewriting live links behind 404 bot walls

## Overview

### Summary
A link that answers 404 or 410 twice is no longer `DEAD` when its host's root answers 404 or 410
too: it becomes `UNVERIFIABLE (404, host root answers 404 too)`, and `--apply` leaves it alone.
A live site's home page doesn't answer 404, so a host that does can't be told apart from a bot
wall. Separately, a permanent redirect that ends in a hop with no `Location` now names its
target in the report. Size: ~140 lines; 0 questions wait under Open for you.

### Problem
rotguard 0.2.0 rewrote 16 links to 3 live www.ftc.gov pages into Wayback snapshots in the first
affilytics CI report (affilytics PR 502). Chrome gets the real pages with status 200. Akamai
answers rotguard's client with one 1301-byte "Page Not Found" page and status 404: for the
pages, for the root https://www.ftc.gov/ and for a made-up path alike. A wrong write is the
worst outcome rotguard has.

rotguard counts a final 404 or 410 as `DEAD` with no other check. The recheck after
`recheck_delay_ms` sends the same request and gets the same wall. The bot-wall check from 0.1
only looks at 401, 403 and 999, so a wall that answers 404 slips through.

In the same report, https://www.edisonresearch.com/the-infinite-dial-2020/ answered 301 to
https://ssrs.com/insights/the-infinite-dial-2020/, a real move, and ssrs.com answered with a
Sucuri 307 that has no `Location`. The report says only `could not check (no location)`, so the
move you could act on is hidden.

### What you can do with it
1. Run `rotguard --apply` on affilytics. The 3 ftc.gov URLs are listed as
   `UNVERIFIABLE 404, host root answers 404 too` (in the PR body,
   `could not check (404, host root answers 404 too)` under the folded list), the posts keep
   their ftc.gov links, and the job doesn't fail for them.
2. Run it on a site whose dead links sit on hosts with a live home page (amazon's
   resource-center, iab.com). They stay `DEAD 404` with a snapshot fix, as in 0.2.0.
3. Read the edisonresearch line as
   `UNVERIFIABLE no location after 301 to https://ssrs.com/insights/the-infinite-dial-2020/`
   and update the link by hand. rotguard still proposes no fix, because it never reached the
   target.

### Not in this version
- Walls that let the root through and 404 only deep paths. The root is the only page every live
  site is known to serve; nothing offline tells such a wall from a dead page.
- Telling a wall from a site gone as a whole. A host that serves 404 everywhere, root included,
  is `UNVERIFIABLE`, not `DEAD`.
- Stripping `utm_*` parameters a redirect added to a fix (www.shareasale.com → awin.com). The
  link with them works, so the write isn't wrong, and a stripped URL is one rotguard never
  fetched. Parked in `docs/backlog.md`.
- A config key to turn the rule off per host. A fix release adds no keys.

### How it works
The change sits in `check_one` in `src/check.rs`, after the recheck, and reuses the per-origin
root fetch that 0.2.0 added for the soft-404 root control. Nothing in apply, report or the exit
code changes: an `UNVERIFIABLE` verdict already proposes no fix, is never written and fails the
job only with `--strict`.

```text
Context: rotguard 0.2.1 (changed parts marked *)

                  ┌──────────────┐
                  │ Site owner   │
                  └──────┬───────┘
  rotguard --apply       │ ▲ report: DEAD | UNVERIFIABLE "404, host root answers 404 too"*
                  ┌──────▼─┴───────────────┐  replaces files (Dead and   ┌────────────────┐
                  │ rotguard               ├────────────────────────────►│ Site directory │
                  │ check.rs: root control*│  Permanent fixes only)      └────────────────┘
                  └──────┬───────────┬─────┘
       GET each link,    │           │ snapshot lookups (dead and suspect links only)
       GET / once per    ▼           ▼
       origin with a 404*  ┌─────────────────┐  ┌──────────────────────────┐
                           │ Linked websites │  │ Wayback Machine CDX API  │
                           └─────────────────┘  └──────────────────────────┘
```

#### The root as the control for a 404
When a link is still `DEAD` after its recheck and the last answer came with a status (not a DNS
failure), rotguard looks at the root of the origin that gave that answer. If the root's chain
ends in 404 or 410, the link becomes `UNVERIFIABLE` with the reason
`<link status>, host root answers <root status> too`. The root is fetched once per origin per
run, through the same per-host gate as every other request, and shared with the soft-404 root
control.

The rule judges by status alone. On the hosts probed, the wall's 404 page and a gone site's 404
page are both the same page at every path, so comparing bodies tells them apart in neither
direction. A body comparison would also put a write behind a similarity threshold, which fails
toward a wrong write when a 404 page embeds the path or a nonce.

#### When the root says nothing
Only positive evidence downgrades a `DEAD`. A root that times out, fails TLS, answers 403, 429 or
5xx, or redirects to a 2xx page leaves the link `DEAD`, as today. A root that 403s while its
paths 404 is a normal file server or bucket (user-images.githubusercontent.com, S3), and treating
it as a wall would hide their real dead files. A root that redirects counts by where it ends: a
redirect to a 404 is a 404.

The root control in the soft-404 check keeps its 0.2.0 meaning: a root that didn't end in 2xx
says nothing there. The shared cache now remembers the root's status, so the wall check can read
it, but the soft-404 check still uses only a 2xx root.

```text
Flow: a link that answers 404 (main path and the failure it fixes)
  1. GET link ──404──► sleep recheck_delay ──► GET link ──404──► classify: Dead(404)
  2. root(origin of the last hop) ──cached? else GET / (follow redirects)──►
       3a. root ends 404|410 ──► UNVERIFIABLE "404, host root answers 404 too" ──► no lookup,
           no write                                       (ftc.gov: the fix for PR 502)
       3b. root ends 2xx ──► DEAD ──► CDX lookup ──► snapshot fix, written with --apply
       3c. root fails, 403, 5xx ──► DEAD, as 3b
  4. deadline hits during 2 ──► UNVERIFIABLE "deadline", as for any unfinished check
```

#### The cost on real hosts
Most dead links sit on hosts with a live home page, and those stay `DEAD`. The ones that change
sit on hosts whose root answers 404 to rotguard's client: the walled www.ftc.gov, a retired
Blogger blog (youtube-eng.googleblog.com, 1 URL in affilytics, which got 429 in CI anyway), a
GitHub Pages account with no user site (rust-lang.github.io), and asset hosts such as
fonts.gstatic.com and i.ytimg.com. Their dead links are listed every week as `UNVERIFIABLE` and
need a hand fix. That is the safe side of the mistake.

The extra cost is 1 root fetch per origin that has a link still dead after its recheck: 1 GET,
or up to 11 if the root redirects 10 times. On affilytics the run gets shorter: 1 root GET
replaces 13 snapshot lookups for the ftc.gov occurrences.

#### No location after a permanent redirect
When a redirect chain fails because a hop has no usable `Location`, and the hops before that one
start with 1 or more 301/308 hops, the reason becomes
`no location after <statuses> to <target>`, for example
`no location after 301 to https://ssrs.com/insights/the-infinite-dial-2020/`. The target is
where the leading 301/308 run ended, the same URL a `PERMANENT REDIRECT` fix would use. The
verdict stays `UNVERIFIABLE` with no fix: the target answered with a challenge, not a page. A
chain through the root keeps its 0.2.0 verdict, `SUSPECT redirect to root`.

### Decisions
#### D1: the host root's status is the control for a 404
- Why: a wall answers the root like any other path, and a live site's home page doesn't answer
  404. Probed with rotguard 0.2.0's client: www.ftc.gov's root answers 404, the roots of
  www.example.com, affiliate-program.amazon.com and www.iab.com don't.
- Rejected: comparing the root's 404 body with the link's. youtube-eng.googleblog.com's root
  and its dead post return identical bodies, like ftc.gov's wall, so it separates nothing on
  the probed hosts, and a threshold that misses fails toward a wrong write.
- Rejected: asking the Wayback Machine how recently it saw the URL as 200. It puts an external
  service inside every verdict, costs 1 lookup per link (about 4 s each), and fails toward a
  write for a walled page the archive rarely crawls.
- Cost if wrong: a dead page on a host whose root 404s gets a weekly `UNVERIFIABLE` line and no
  fix. Undoing it is the 10-line branch in `check_one`, or a per-host key in a minor release.

#### D2: an inconclusive root keeps the link DEAD
- Why: a root that fails or answers 403 or 5xx says nothing about the page. Downgrading on it
  would hide real dead files on 403-root file servers and turn a 1-off root timeout into a
  week of hidden dead links.
- Rejected: any root answer other than 2xx downgrades. Safer against walls that 403 the root
  and 404 the paths, a shape not seen on any probed host.
- Cost if wrong: a wall of that shape still gets rewritten; adding 401/403/999 to the root rule
  is a 1-line change.

#### Smaller decisions
- D3: the root comes from the origin of the link's last hop, because that host gave the 404. A
  cross-host redirect into a wall is caught; changing it means passing a different URL.
- D4: the root check runs after the recheck, because a 404 that recovers on recheck needs no
  root. Running it during the recheck sleep would save time and is a later optimisation.
- D5: the rule applies to hosts with `soft404 = false` too, because it is about `DEAD`, not
  soft-404s.
- D6: the no-location target appears only in the reason text, because a fix needs a fetched
  target. A new JSON field would be an add for a minor release.

### Assumptions
- A1: the 404-wall link's class is `UNVERIFIABLE`, not `BLOCKED`. The user chose it in the
  interview, so a site gone as a whole stays listed for a hand check.
- A2: the architect's option A (root status alone) is the design. Options B (body escape) and C
  (body match) recover no URL on the user's data and add a wrong-write window or fail toward
  writes.
- A3: the no-location report fix is in this release. It is about 10 lines, a reporting fix, and
  rotguard already had the hop it dropped.
- A4: the `utm_*` stripping is parked, not built. It changes what `--apply` writes, needs a
  never-fetched URL, and is an add, not a fix.
- A5: the reason text is `<status>, host root answers <status> too`, and reports in every format
  show it through the existing `UNVERIFIABLE` rendering, with no new JSON field.
- A6: in the acceptance-test fixture an unmatched `/` answers a 200 home page instead of 404, so
  the existing `DEAD` scenarios keep a live root; wall tests route `/` to 404 themselves.
- A7: the version bump, the release and the affilytics rerun happen in another session after the
  merge; this design changes only `CHANGELOG.md`'s `[Unreleased]` section among release files.

### Scope
- Root control for 404s in `src/check.rs` (`RootPage` keeps the root's final status, `root()`
  stores it, the branch in `check_one`): ~35 lines, for the 3 ftc.gov URLs (16 occurrences) in
  affilytics.
- No-location reason after a permanent run in `classify`: ~10 lines, for the edisonresearch link.
- README class table, `docs/architecture.md`, `CHANGELOG.md` `[Unreleased]`: ~25 lines.
Size: ~140 (70 estimated × 2.0, median of the last 5 cross-repo design builds)
Touches: none — the change is in `src/check.rs`, tests and docs; no risk path is involved.

### Open for you
none

## Spec

### Requirements
- R1: WHEN a link's final answer is 404 or 410 on both tries AND the root of the origin of the
  link's last hop ends in 404 or 410, THE SYSTEM SHALL classify the link `UNVERIFIABLE` with the
  reason `<link status>, host root answers <root status> too`, look up no snapshot for it and
  propose no fix.
- R2: WHEN a link's final answer is 404 or 410 on both tries AND that root ends in 2xx, THE
  SYSTEM SHALL classify the link `DEAD` as 0.2.0 does.
- R3: IF the root fetch fails (timeout, connection, TLS, DNS, redirect error) or the root ends in
  any status other than 2xx, 404 or 410, THEN THE SYSTEM SHALL classify the link `DEAD` as 0.2.0
  does.
- R4: THE SYSTEM SHALL judge the root by the final status of its redirect chain, following
  redirects as for a link.
- R5: THE SYSTEM SHALL fetch each origin's root at most once per run, sharing it between the
  404 check and the soft-404 root control, and only for an origin that has a link still dead
  after its recheck or a soft-404 candidate.
- R6: THE SYSTEM SHALL apply R1 to hosts with `soft404 = false` as well.
- R7: WHEN a redirect chain fails because a hop has no usable `Location`, AND the hops before
  that hop start with 1 or more 301/308 hops (the hop with no `Location` is never part of the
  run, whatever its status), AND the chain doesn't pass through the root on a host with soft-404
  detection on, THE SYSTEM SHALL classify the link `UNVERIFIABLE` with the reason
  `no location after <statuses of the 301/308 run, space-separated> to <URL the run reached>`
  and propose no fix.
- R8: WHEN a redirect chain fails because a hop has no usable `Location`, AND the hops before
  that hop don't start with a 301/308 hop, AND the chain doesn't pass through the root on a host
  with soft-404 detection on (that chain stays `SUSPECT redirect to root`), THE SYSTEM SHALL keep
  the reason `no location`.

### Acceptance scenarios
- S1 [R1]: Given host M answers `/` and every other path with status 404 and the same body, and
  `post.md` links `M/guides/page`, when `rotguard --apply` runs, then the entry reads
  `UNVERIFIABLE 404, host root answers 404 too`, `post.md` is byte-for-byte unchanged, the
  archive mock gets no request, the summary counts dead 0 and unverifiable 1, and the exit code
  is 0.
- S2 [R1]: Given the S1 host, when `rotguard --apply --strict` runs, then `post.md` is unchanged
  and the exit code is 1.
- S3 [R1]: Given the S1 host, when `rotguard --format markdown` runs, then the report contains
  `could not check (404, host root answers 404 too)` for `M/guides/page`.
- S4 [R1]: Given host M answers `/` with 410 and `/gone` with 404, when rotguard runs, then the
  entry for `M/gone` reads `UNVERIFIABLE 404, host root answers 410 too`.
- S5 [R1]: Given host M answers every path, `/` included, with 404, and `post.md` links `M/`
  itself, when `rotguard --apply` runs, then the entry reads
  `UNVERIFIABLE 404, host root answers 404 too` and `post.md` is unchanged.
- S6 [R2]: Given host M answers `/` with a 200 page and `/gone` with 404, and the archive has a
  capture of `M/gone`, when `rotguard --apply` runs, then the entry reads `DEAD 404`, `post.md`
  links the snapshot, and the exit code is 0.
- S7 [R3]: Given host M answers `/` with 503 every time and `/gone` with 404, and the archive has
  a capture of `M/gone`, when `rotguard --apply` runs, then the entry reads `DEAD 404` and
  `post.md` links the snapshot.
- S8 [R3]: Given host M answers `/` with 403 and `/gone` with 404, when rotguard runs, then the
  entry for `M/gone` reads `DEAD 404`.
- S9 [R3]: Given host M's `/` waits longer than `timeout_ms` and `/gone` answers 404, when
  rotguard runs with a deadline long enough for the retries, then the entry for `M/gone` reads
  `DEAD 404`.
- S10 [R4]: Given host M answers `/` with 301 to `/home`, `/home` with 404 and `/gone` with 404,
  when rotguard runs, then the entry for `M/gone` reads
  `UNVERIFIABLE 404, host root answers 404 too`.
- S11 [R4]: Given host M answers `/` with 302 to `/home`, `/home` with a 200 page and `/gone` with
  404, when rotguard runs, then the entry for `M/gone` reads `DEAD 404`.
- S12 [R1]: Given host M answers `/old` with 301 to `/gone` on a second local server W, and W
  answers `/` and every path with 404, when rotguard runs, then the entry for `M/old` reads
  `UNVERIFIABLE 404, host root answers 404 too`.
- S13 [R5]: Given host M answers `/` and every path with 404 and `post.md` links `M/a`, `M/b`
  and `M/c`, when rotguard runs, then all 3 entries read
  `UNVERIFIABLE 404, host root answers 404 too` and M's request log shows exactly 1 request for
  `/`.
- S14 [R5]: Given host M answers `/` with a 200 page and `/ok` with a 200 page and no link on M
  answers 404 or looks like a soft-404, when rotguard runs, then M's request log shows no
  request for `/`.
- S15 [R6]: Given host M is listed with `soft404 = false` and answers `/` and every path with
  404, when rotguard runs on a link to `M/gone`, then the entry reads
  `UNVERIFIABLE 404, host root answers 404 too`.
- S16 [R7]: Given host M answers `/the-infinite-dial-2020/` with 301 to
  `/insights/the-infinite-dial-2020/`, and that path with 307 and no `Location` header, when
  `rotguard --apply` runs, then the entry reads
  `UNVERIFIABLE no location after 301 to M/insights/the-infinite-dial-2020/`, there is no fix
  line, and `post.md` is unchanged.
- S17 [R7]: Given host M answers `/a` with 308 to `/b`, `/b` with 301 to `/c`, and `/c` with 307
  and no `Location`, when rotguard runs, then the entry reads
  `UNVERIFIABLE no location after 308 301 to M/c`.
- S18 [R8]: Given host M answers `/a` with 307 and no `Location` header, when rotguard runs, then
  the entry reads `UNVERIFIABLE no location`.
- S19 [R8]: Given host M answers `/a` with 302 to `/b` and `/b` with 307 and no `Location`
  header, when rotguard runs, then the entry reads `UNVERIFIABLE no location`.
- S20 [R7]: Given host M answers `/x` with 301 to `/y` and `/y` with 301 and no `Location`
  header, when rotguard runs, then the entry reads `UNVERIFIABLE no location after 301 to M/y`.
- S21 [R5]: Given host M answers `/` with a 200 page titled "Home", `/a/page` and every other
  path under `/a/` with the same 200 page titled "Same", and `/gone` with 404, when rotguard runs
  on links to `M/a/page` and `M/gone`, then the entry for `M/a/page` reads
  `SUSPECT probe: same title`, the entry for `M/gone` reads `DEAD 404`, and M's request log
  shows exactly 1 request for `/`.

### Details
**Types in `src/check.rs`.** `RootPage` gets 3 variants:
- `Usable { title, shingles }`: the root's chain ended in 2xx (unchanged).
- `Status(u16)`: the chain ended in any other status.
- `Failed`: the fetch returned an error.

`Probes::reason` matches only `Usable`, so the soft-404 root control behaves as in 0.2.0. A new
`Probes::root_status`-style read, or a match on the same `Arc<RootPage>`, gives the 404 check
the status.

**The branch in `check_one`.** After `let verdict = classify(&outcome, host);` and before the
bot-wall branch:

```text
if outcome is Ok(fetched) and verdict is Dead and last hop exists:
    root = probes.root(fetcher, &last.url)          // origin of the last hop, cached
    if root is Status(404 | 410):
        return Unverifiable { reason: "{last.status}, host root answers {root} too" }
return verdict (continue as today)
```

A DNS `Dead` has no fetched chain and is skipped. The branch runs inside the existing
`timeout_at(deadline, check_one)`, so a deadline during the root GET gives
`Unverifiable("deadline")`.

`root()` keeps its target rule: the URL with path `/`, no query, no fragment, fetched with
`fetcher.fetch` (redirects followed by hand, up to 10, no retries). The cache key stays
`url.origin().ascii_serialization()`; the soft-404 control calls it with the link URL and the
404 check with the last hop's URL, which share an origin unless the link was redirected across
hosts.

**The no-location reason in `classify`.** In the `Err(FetchError::NoLocation(chain))` case that
doesn't match `redirects_to_root`: let `n` be the count of leading 301/308 hops in
`chain[..chain.len() - 1]` (the last hop is the one with no `Location` and is never part of the
run). When `n >= 1`, the reason is
`format!("no location after {} to {}", statuses(&chain[..n]), chain[n].url)` with statuses
space-separated as in `PERMANENT REDIRECT 301 200`; `chain[n]` always exists because the run
excludes the last hop. Otherwise it stays `no location`. `Loop` and
`TooManyHops` are unchanged. The markdown report escapes the reason through `md_escape` as for
any `UNVERIFIABLE`.

**Test fixture.** `tests/acceptance/support.rs`: `Router::respond` answers an unmatched `/` with
`page("Home", "welcome")`, so scenarios that never route `/` keep a live root and stay `DEAD`.
A scenario that needs a 404 root routes `/` explicitly; routes match first. Unit tests in
`src/check.rs` that expect `Dead` add a 200 `/` mock, mounted with `.with_priority(1)` where
the server already has a catch-all mock (`scripted()` mounts one first, and wiremock picks the
first-mounted match at equal priority), as in `a_404_twice_is_dead_after_the_recheck_delay`.
S12 starts a second wiremock server for W in the test.

**Docs.**
- README class table: `DEAD` reads "404, 410 or a host DNS says doesn't exist, on two tries,
  on a host whose home page doesn't answer 404 or 410 too"; `UNVERIFIABLE` adds "a 404 or 410
  on a host whose home page answers 404 or 410 too (a bot wall can't be told apart)".
- `docs/architecture.md`: the Linked websites bullet, the soft-404 and bot-wall paragraph, the
  Soft-404 judgement hard part (the root control now also downgrades a `Dead`, never upgrades),
  and a Decisions line citing this design's D1 and D2.
- `CHANGELOG.md` `[Unreleased]`: 1 bullet for the 404 rule (naming the request it may add) and 1
  for the no-location reason.

- **Facts**:
  - probed: rotguard 0.2.0 release build on a scratch site, www.ftc.gov `/`,
    `/business-guidance` and a made-up path all answer 404 to its client (curl with the same
    User-Agent gets 403, so the answer depends on the TLS client).
  - probed: same build, roots of www.example.com, affiliate-program.amazon.com and www.iab.com
    don't answer 404; their dead affilytics links stay `DEAD` under R2.
    rdiachenko.github.io's root answers 301 to rdiachenko.com then 200, so a missing project
    under it stays `DEAD`.
  - probed: same build, roots of youtube-eng.googleblog.com, rust-lang.github.io,
    fonts.gstatic.com and i.ytimg.com answer 404; dead links there become `UNVERIFIABLE`.
  - probed: curl, youtube-eng.googleblog.com's root and its dead post return byte-identical
    bodies (same sha1), so a body comparison wouldn't separate it from the ftc.gov wall.
  - probed: same build, www.edisonresearch.com/the-infinite-dial-2020/ ends
    `UNVERIFIABLE no location` through ssrs.com's 307 with no `Location`.
- **Invariants**:
  - Only a root that ended in 404 or 410 changes a verdict, and only from `Dead` to
    `Unverifiable`; owner: `check_one`. No root answer ever turns a link into `Dead` or adds a fix.
  - The soft-404 root control reads only `RootPage::Usable`; owner: `Probes::reason`.
  - Every root fetch runs inside the per-URL `timeout_at(deadline, ...)`; owner: `check_all`.
- **Budgets**: 1 root fetch per origin with a still-dead link: up to 11 GETs (10 redirects) of
  at most `timeout_ms` (default 10 s) each, each after a `host_interval_ms` gate wait on its host,
  all bounded by the per-URL `timeout_at(deadline)`. Usually 1 GET; unverified on a large site;
  on affilytics it replaces 13 snapshot lookups.

### Risk paths
As in `.whittle/gates.toml`; this design touches none of them.

### Fitness rules
As in `.whittle/gates.toml`: only `src/apply.rs` writes files, tests never hit the real network
(S12's second server is local), no private data in tracked files.

### Review changes
- Round 1: R7, the overview and Details count the 301/308 run without the hop that has no
  `Location`, so a run ending in a 301/308 with no `Location` names its target (S20).
- Round 1: R8 keeps the soft-404 root exception, matching R7 and today's `SUSPECT redirect to root`.
- Round 1: the root-fetch budget counts redirects (up to 11 GETs) and gate waits.
- Round 1: S21 checks that the 404 check and the soft-404 root control share 1 root fetch.
- Round 1: the unit-test recipe mounts the `/` mock with `.with_priority(1)` over `scripted()`'s
  catch-all.
