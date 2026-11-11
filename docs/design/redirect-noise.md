---
status: approved
mode: epic
---
# Keep a redirect's noise out of the site

## Overview

### Summary
A permanent redirect that only adds query parameters or a locale path segment to the link is no
longer a fix: the link is `OK` and `--apply` keeps it as written. A real move followed by such a
step is rewritten to the URL before it. A real move whose target carries `utm_*` parameters the
redirect added is rewritten without them, after 1 request shows the stripped URL answers with
the same page title. A chain with none of these is rewritten as in 0.2.1.
Size: ~200 lines; 0 questions wait under Open for you.

### Problem
`--apply` promises to write only fixes that can't be wrong. In the first real fix PR (affilytics
PR 502, rotguard 0.2.1) you reverted 2 of the 10 automatic rewrites by hand, and a 3rd wrong one
appears when rotguard runs from a UK machine. A wrong write costs more trust than a missed fix.

rotguard 0.2.1 rewrites a link to wherever its leading 301/308 hops end, whatever the redirect
added on the way. The 3 wrong rewrites, reproduced today with the 0.2.1 binary:

1. success.awin.com 301s every article to itself plus `?language=en_US`. The link you wrote is
   the clean one.
2. www.shareasale.com 301s to
   `https://www.awin.com/us/getting-started-sas?utm_medium=redirect&utm_source=www.shareasale.com`.
   The move is real, the 2 parameters are the redirect's own tracking.
3. newsroom.pinterest.com 301s `/news/amazon-storefront-linking/` to `/en-gb/news/…` from the UK
   and to another locale elsewhere, so the same site gets a different "fix" depending on where
   rotguard runs.

### What you can do with it
1. Run `rotguard --apply` on affilytics. The success.awin.com link and the Pinterest link (on
   3 pages) stay as you wrote them, don't appear in the report, and don't fail the job, from a
   UK laptop and from the GitHub runner alike.
2. In the same run, `https://www.shareasale.com` becomes
   `https://www.awin.com/us/getting-started-sas`, with no `utm_*` parameters.
3. The other 8 permanent redirects 0.2.1 finds there today, the MDN, linktr.ee, prettylinks,
   hls.harvard.edu and amzwatcher links among them, are rewritten exactly as 0.2.1 rewrites
   them.
4. After the release, drop the `ignore` entries for these links from the affilytics blog's
   `rotguard.toml`, so rotguard checks the links again. That is a follow-up in that repo, not
   part of this change.

### Not in this version
- A row in the report for a kept link. It is plain `OK`. A row would need a new `class` value,
  which the 0.x contract doesn't allow lightly. Parked in `docs/backlog.md`.
- A chain where noise comes first and a real step follows, as on Zendesk help centres
  (`/hc/articles/N` → `/hc/en-us/articles/N` → `/hc/en-us/articles/N-slug`). The link is kept
  as written and the slug is not added. Parked.
- A single hop that changes the host or scheme and adds noise at once
  (`http://a/x` → `https://a/en-gb/x`). It is still rewritten as in 0.2.1. Not seen yet. Parked.
- A locale segment replaced (`/en/x` → `/en-US/x`) or removed (linktr.ee). Both are rewritten as
  in 0.2.1: a link that names a locale gets the same answer on every run.
- Tracking parameters other than `utm_*`, and any parameter the link itself had.
- A `[hosts."<name>"]` key to turn the rule off. Every mistake the rule can make keeps a link
  that still works, and 1 hand edit settles that link for good.
- The version bump, the release and the affilytics follow-up. They happen in another session.

### How it works
The change sits in `src/check.rs`: `classify` decides where a permanent fix points, and
`check_one` gains 1 last step for `utm_*`. A kept link is the existing `OK` verdict, so the
report, the exit-code rule, the JSON fields, `rotguard.toml` and `src/apply.rs` don't change.

```text
Context: rotguard (changed parts marked *)

                  ┌──────────────┐
                  │ Site owner   │
                  └──────┬───────┘
  rotguard --apply       │ ▲ report: a noise redirect is OK*, a fix has no added utm_**
                  ┌──────▼─┴───────────────┐  replaces files (Dead and   ┌────────────────┐
                  │ rotguard               ├────────────────────────────►│ Site directory │
                  │ check.rs: noise rule*, │  Permanent fixes only)      └────────────────┘
                  │ utm strip*             │
                  └──────┬───────────┬─────┘
       GET each link;    │           │ snapshot lookups (dead and suspect links only)
       GET a fix without ▼           ▼
       its added utm_**  ┌─────────────────┐  ┌──────────────────────────┐
                         │ Linked websites │  │ Wayback Machine CDX API  │
                         └─────────────────┘  └──────────────────────────┘
```

#### What counts as noise
rotguard judges each redirect step, from one URL to the next, on its own. A step is noise when
both URLs have the same scheme, host and port, the new URL keeps every query parameter the old
one had, and the only other change is one of these:

- Parameters added or reordered (`?language=en_US`). A trailing slash may change with added
  ones.
- 1 locale-shaped path segment inserted anywhere in an otherwise identical path (`/en-gb/`,
  `/de/`, `/zh-Hans/`). A trailing slash may change with it.

Locale-shaped means 2 letters, optionally followed by `-` or `_` and 2 letters, 4 letters or 3
digits. `Reference` (MDN) and `plans` (prettylinks) are not, so those steps are real moves. A
step that only adds a trailing slash, changes or drops a parameter, or removes a segment is a
real move too.

#### Why the URL's shape decides, and not a second request
A second request with another `Accept-Language` can expose a redirect that follows the header,
and Pinterest's does: `de` gives `/de/news/…`. But with no header, `en-US` or a nonsense value,
Pinterest falls back to where the request comes from. A site that uses only the location looks
canonical to any request rotguard can send from 1 machine, so the extra request can't make the
rewrite safe. It would cost 1 request per such link on every run.

The shape rule errs to one side. When a site really moved every page under `/en/` for everyone,
rotguard keeps your link, and the link still works through the redirect. That is a missed
tidy-up, not a wrong write.

#### Where the fix points
rotguard walks the leading 301/308 steps from the link and stops at the first noise step. The
URL it has reached is the fix. If it hasn't moved, the first step was noise and the link is
`OK`. If no step is noise, the fix is the end of the 301/308 run, as in 0.2.1.

So `http://` → `https://` → `?language=en_US` is rewritten to the `https://` URL, which the
chain itself proved: 0 extra requests. On the next run that URL's first step is noise, so
rotguard leaves it alone.

One verdict gets stricter. When the URL before the noise is the site root and the link isn't
(`/promo` → `/` → `/en-gb/`), the link is `SUSPECT redirect to root`, like any other redirect to
the home page. 0.2.1 wrote `/en-gb/` for it.

#### Dropping the tracking parameters a redirect added
When a fix has `utm_*` parameters whose names the link's own query doesn't have, rotguard
removes them and GETs the result once. It writes the stripped URL only when that URL answers as
an `OK` link would (a 2xx, directly or through a redirect whose first step is noise) and its
page has the same `<title>` as the page the link ended on. This is the first fix no server named, which is why it
needs proof.

When the proof fails (a 404, another redirect, a different title, a timeout), the link is kept
as written and counted `OK`: it still works through the redirect, and the only working target
carries tracking you don't want in your pages. The next run tries again.

```text
Flow: a link whose 301/308 hops end in a 2xx page
  1. GET link, follow hops ──► chain: link ─301/308─► hop ─► … ─► page (2xx)
  2. classify walks the 301/308 steps and stops at the first noise step
       2a. the first step is noise ──► OK, kept as written        (awin, Pinterest: 0 requests)
       2b. n ≥ 1 real steps first ──► PERMANENT REDIRECT, fix = the URL after step n
                                                                  (MDN, linktr.ee: 0 requests)
  3. soft-404 checks on the link, as in 0.2.1 (they can still make 2a or 2b SUSPECT)
  4. the fix has utm_* names the link lacks? no ──► done
       yes ──► GET the fix without them (1 request plus its hops, no retry)
       4a. answers as an OK link, same <title> ──► fix = the stripped URL     (shareasale)
       4b. anything else ──► OK, kept as written
       4c. deadline_s passes during the GET ──► UNVERIFIABLE deadline, nothing written
```

#### What it costs
Keeping a link and picking an earlier hop cost 0 requests. The strip costs 1 GET for each link
whose fix carries added `utm_*`: 1 of the 241 links on affilytics. That GET is paced and queued
like every other request to its host, so it waits behind that host's other links, and
`deadline_s` bounds it. Once the stripped URL is written, later runs don't repeat it.

A kept link pays its redirect hops on every run, where a rewritten link would pay 1 request.
On affilytics that is 3 extra GETs per run: 1 for the success.awin.com link and 2 for the
Pinterest link.

Run today from a UK machine, 0.2.1 finds 11 permanent redirects among affilytics' 241 links.
The new rule keeps 2 of them, strips 1 and leaves the other 8 fixes unchanged.

### Decisions
#### D1: the shape of each redirect step decides what is noise
- Why: nothing rotguard can send from 1 machine tells a location-driven redirect from a
  canonical one, and every mistake of the shape rule keeps a link that still works. Probed
  today: Pinterest's target follows `Accept-Language` and falls back to the request's location;
  MDN's and linktr.ee's targets don't vary.
- Rejected: a second request with another `Accept-Language` for locale steps. It writes a
  location-only locale, the one error this change exists to stop, and costs 1 request per kept
  link per run.
- Rejected: comparing only the link with the final target. It writes
  `/hc/en-us/articles/N-slug` on Zendesk, where the locale step is followed by a real one.
- Cost if wrong: a real move that looks like noise is never tidied. When the site retires the
  redirect the link turns `DEAD` and gets a snapshot although the page lives on. Undoing it is
  1 function, `is_noise`, in `src/check.rs`.

#### D2: a kept link is plain OK, with no report row
- Why: the JSON `class` values are a published list, and a new one breaks a reader that
  switches on them. success.awin.com redirects every article, so a row per kept link would
  repeat in every weekly PR.
- Rejected: a row in the folded "no action needed" list naming the redirect. Useful to see what
  was kept, but it needs the new class.
- Cost if wrong: you can't see from the report which links were kept. Adding a row later is an
  add in a minor release.

#### D3: only `utm_*` is stripped, and only with proof
- Why: `utm_*` is the only tracking a redirect was seen to add, and it exists for analytics,
  though a site can vary a page's text by it. Click IDs (`gclid`, `fbclid`) are added by ad
  clicks, not by a server's redirect, and `ref` selects content on some sites. Probed today: the
  stripped awin URL answers 200 with the same title and the same visible text.
- Rejected: writing the target with its tracking when the proof fails. It puts the noise you
  reverted by hand back into your pages.
- Rejected: a 2xx alone as proof. A site can answer 200 with a landing page when parameters are
  missing; the title check costs 0 requests.
- Cost if wrong: a page that personalises its text by `utm_*` under an unchanged title gets
  past the proof, and your link then shows the page's default text. A wrong strip lands in the
  fix PR and only review catches it. The list is 1 constant.

#### Smaller decisions
- D4: no `[hosts]` key, because a kept link you disagree with is settled by 1 hand edit, and a
  key name becomes part of the 0.x contract. Adding one later is a minor release.
- D5: a deadline during the strip request gives `UNVERIFIABLE deadline`, like any unfinished
  check. The file is untouched either way and the next run retries.
- D6: strip requests are not shared between links, because each distinct link is checked once
  and 2 links rarely share a target. A cache is about 15 lines if a site needs it.
- D7: a kept link still goes through the soft-404 checks, because a redirect that only adds a
  parameter can still land on a "not found" page.

### Assumptions
- A1: the architect's option A is the design: the per-step shape rule plus the verified `utm_*`
  strip. Its option B (an `Accept-Language` request) can write a location-only locale, and
  option C (no strip) leaves the shareasale move unmade.
- A2: a kept link reports as plain `OK`. The idea asked "plain OK, or a row"; the architect
  recommends `OK` for every keep, a failed strip included, because of the `class` contract.
- A3: no `[hosts."<name>"]` key. The idea asked whether one earns its place; see D4.
- A4: tracking means `utm_*` only, matched without regard to case. The only evidence is 1
  shareasale link.
- A5: when the stripped URL can't be proven, the link is kept as written, following "when in
  doubt, don't rewrite".
- A6: a chain with noise first and a real step after is kept as written. Zendesk's chain,
  probed today, is the example; affilytics has no such link (its 4 `/hc/` links already name
  `en-us`).
- A7: the 2 backlog lines this covers are removed by the build, not now, because the idea says
  "removed when it ships".
- A8: the release, the version number and the affilytics `ignore` follow-up happen in another
  session. This design changes only the `[Unreleased]` section of `CHANGELOG.md` among release
  files.
- A9: a fragment in a hop's URL is ignored when judging noise, and fragments in a fix are
  handled as in 0.2.1: the target's own fragment wins, else the link's is kept.

### Scope
- Noise rule in `src/check.rs` (`is_noise`, `locale_shaped`, the fix choice and the root test in
  `classify`): ~55 lines, for the success.awin.com link and the Pinterest link (3 pages) on
  affilytics.
- `utm_*` strip in `src/check.rs` (`without_tracking`, the last step of `check_one`, the title
  check): ~45 lines, for the shareasale link.
- README class table, `docs/architecture.md`, `docs/config.md`, `CHANGELOG.md` `[Unreleased]`,
  the 2 backlog lines: ~30 lines.

Size: ~200 (130 estimated × 1.5, median of the last 3 rotguard design builds)
Touches: none — the change is in `src/check.rs`, tests and docs; no risk path is involved.

### Open for you
none

## Spec

### Requirements
- R1: THE SYSTEM SHALL treat a redirect step from URL U to URL V as a noise step only when U and
  V have the same scheme, host and port, every decoded query pair of U occurs in V at least as
  many times as in U (order ignored), and one of these holds: (a) the paths are identical;
  (b) V's path segments are U's with exactly 1 locale-shaped segment inserted at any position, a
  trailing-slash difference ignored; (c) the paths differ only by a trailing slash and V has at
  least 1 query pair more than U. Fragments are ignored.
- R2: THE SYSTEM SHALL treat a path segment as locale-shaped only when it is 2 ASCII letters,
  optionally followed by `-` or `_` and then 2 ASCII letters, 4 ASCII letters or 3 digits, in
  any letter case.
- R3: WHEN a link's chain ends in 2xx after a leading run of 1 or more 301/308 hops AND the first
  step of that run is a noise step, THE SYSTEM SHALL classify the link `OK`, propose no fix and
  write nothing for it.
- R4: WHEN a link's chain ends in 2xx after a leading run of 301/308 hops AND the run starts with
  n ≥ 1 steps that are not noise steps, THE SYSTEM SHALL classify the link `PERMANENT REDIRECT`
  with the fix set to the URL reached after the last step before the run's first noise step, or
  to the end of the run when no step is a noise step.
- R5: WHEN the fix R4 would propose for a link whose path is not `/` has the path `/`, on a host
  with soft-404 detection on, THE SYSTEM SHALL classify the link `SUSPECT` with the reason
  `redirect to root`, and SHALL keep 0.2.1's root rule for the last hop and the end of the run.
- R6: WHEN a link is `PERMANENT REDIRECT` after the soft-404 checks AND its fix has 1 or more
  query pairs whose key starts with `utm_` (ASCII case ignored) and is not a key of the link's
  own query, THE SYSTEM SHALL GET the fix with those pairs removed (the stripped URL) once,
  following redirects, with no retry, through the same per-host pacing as every request.
- R7: WHEN the stripped URL's chain classifies as `OK` (a 2xx with no redirect, or a chain R3
  keeps; the root rule applies as on a host with soft-404 detection on; no soft-404 probe is
  sent for the stripped URL) AND the title of its final page equals the title of the page the
  link's chain ended on, THE SYSTEM SHALL set the fix to the stripped URL, handling fragments as
  0.2.1 does.
- R8: IF the stripped URL's fetch fails, classifies as anything other than `OK`, or ends on a
  page with a different title, THEN THE SYSTEM SHALL classify the link `OK`, propose no fix and
  write nothing for it.
- R9: THE SYSTEM SHALL remove from a fix only the pairs R6 names, and keep every other pair
  byte for byte and in order; a fix whose query becomes empty has no `?`.
- R10: THE SYSTEM SHALL send no request to a linked site for R3, R4 and R5 beyond those 0.2.1
  sends, and no stripped-URL request for a link whose fix has no pair R6 names. A link R5 makes
  `SUSPECT` gets the snapshot lookup every suspect gets.
- R11: THE SYSTEM SHALL count a link kept under R3 or R8 in the summary's `ok` and not in
  `permanent`, list it in no report format, and let it change no exit code, with or without
  `--strict`.
- R12: THE SYSTEM SHALL run 0.2.1's soft-404 checks on a link kept under R3, so a kept link can
  still be `SUSPECT` or `UNVERIFIABLE`.
- R13: THE SYSTEM SHALL apply R6 to R9 on hosts with `soft404 = false` as well.
- R14: IF `deadline_s` passes while the stripped URL's fetch is running, THEN THE SYSTEM SHALL
  classify the link `UNVERIFIABLE` with the reason `deadline` and write nothing for it.

### Acceptance scenarios
All hosts are the mock site M. "Kept" means: the report has no entry for the link, `post.md` is
byte-for-byte unchanged, the summary counts ok 1 and permanent 0, and the exit code is 0.

- S1 [R1, R3, R10, R11]: Given `/s/article/guide` answers 301 to `/s/article/guide?language=en_US`
  and that URL answers a 200 page, when `rotguard --apply` runs on a link to `M/s/article/guide`,
  then the link is kept and M's log shows exactly 2 requests for `/s/article/guide`.
- S2 [R1, R3]: Given `/a?id=5` answers 301 to `/a?id=5&language=en_US` (200), when
  `rotguard --apply` runs on a link to `M/a?id=5`, then the link is kept.
- S3 [R1, R3]: Given `/a?b=1&a=2` answers 301 to `/a?a=2&lang=en&b=1` (200), when
  `rotguard --apply` runs on a link to `M/a?b=1&a=2`, then the link is kept.
- S4 [R1, R4]: Given `/a?id=5` answers 301 to `/a?id=6` (200), when `rotguard --apply` runs on a
  link to `M/a?id=5`, then the entry reads `PERMANENT REDIRECT 301 200` with `fix: M/a?id=6` and
  `post.md` links `M/a?id=6`.
- S5 [R1, R4]: Given `/a?id=5&x=1` answers 301 to `/a?id=5` (200), when `rotguard --apply` runs
  on a link to `M/a?id=5&x=1`, then `post.md` links `M/a?id=5`.
- S6 [R1, R2, R3]: Given `/news/storefront/` answers 301 to `/en-gb/news/storefront`, which
  answers 301 to `/en-gb/news/storefront/` (200), when `rotguard --apply` runs on a link to
  `M/news/storefront/`, then the link is kept.
- S7 [R1, R2, R4, R10]: Given `/en-US/docs/Web/HTTP/Status/302` answers 301 to
  `/en-US/docs/Web/HTTP/Reference/Status/302` (200), when `rotguard --apply` runs, then
  `post.md` links `M/en-US/docs/Web/HTTP/Reference/Status/302` and M's log shows exactly 1
  request for that path.
- S8 [R1, R4]: Given `/help/en/articles/8619620` answers 308 to `/help/articles/8619620`, which
  answers 308 to `/help/articles/8619620-about-links` (200), when `rotguard --apply` runs, then
  `post.md` links `M/help/articles/8619620-about-links`.
- S9 [R1, R2, R4]: Given `/pricing/` answers 301 to `/pricing/plans/` (200), when
  `rotguard --apply` runs, then `post.md` links `M/pricing/plans/`.
- S10 [R1, R4]: Given `/pricing` answers 301 to `/pricing/` (200), when `rotguard --apply` runs,
  then `post.md` links `M/pricing/`.
- S11 [R1, R2, R3]: Given `/help/articles/9` answers 301 to `/help/de/articles/9` (200), when
  `rotguard --apply` runs, then the link is kept.
- S12 [R1, R2, R3]: Given `/` answers 301 to `/en-gb/` (200), when `rotguard --apply` runs on a
  link to `M/`, then the link is kept.
- S13 [R2, R3]: Given `/docs/x` answers 301 to `/docs/zh-Hans/x` (200), when `rotguard --apply`
  runs, then the link is kept.
- S14 [R2, R4]: Given `/docs/x` answers 301 to `/docs/eng/x` (200), when `rotguard --apply` runs,
  then `post.md` links `M/docs/eng/x`.
- S15 [R1, R3]: Given `/a` answers 301 to `/a/?lang=en` (200), when `rotguard --apply` runs,
  then the link is kept.
- S16 [R4, R10]: Given `/old/x` answers 301 to `/new/x`, `/new/x` with no query answers 301 to
  `/new/x?language=en_US`, and that URL answers a 200 page, when `rotguard --apply` runs on a
  link to `M/old/x`, then the entry reads `PERMANENT REDIRECT 301 301 200` with `fix: M/new/x`,
  `post.md` links `M/new/x`, and M's log shows exactly 2 requests for `/new/x`.
- S17 [R3]: Given `/hc/articles/44` answers 301 to `/hc/en-us/articles/44`, which answers 301 to
  `/hc/en-us/articles/44-getting-started` (200), when `rotguard --apply` runs on a link to
  `M/hc/articles/44`, then the link is kept.
- S18 [R5]: Given `/promo` answers 301 to `/`, `/` answers 301 to `/en-gb/` and `/en-gb/` answers
  a 200 page, when `rotguard --apply` runs on a link to `M/promo`, then the entry reads
  `SUSPECT redirect to root` and `post.md` is unchanged.
- S19 [R6, R7, R9]: Given `/old` answers 301 to
  `/us/getting-started?utm_medium=redirect&utm_source=old`, and `/us/getting-started` answers the
  same 200 page titled "Now Awin" with and without a query, when `rotguard --apply` runs on a
  link to `M/old`, then the entry reads `PERMANENT REDIRECT 301 200` with
  `fix: M/us/getting-started`, `post.md` links `M/us/getting-started`, the exit code is 0, and
  M's log shows exactly 2 requests under `/us/`: 1 for `/us/getting-started` with its 2 `utm_*`
  pairs and 1 for it with no query.
- S20 [R6, R9]: Given `/old?utm_source=mine` answers 301 to
  `/new?utm_source=mine&utm_medium=redirect`, and `/new` answers the same 200 page for any
  query, when `rotguard --apply` runs on a link to `M/old?utm_source=mine`, then `post.md` links
  `M/new?utm_source=mine`.
- S21 [R9]: Given `/old` answers 301 to `/new?id=7&utm_source=old&ref=x`, and `/new` answers the
  same 200 page for any query, when `rotguard --apply` runs on a link to `M/old`, then `post.md`
  links `M/new?id=7&ref=x`.
- S22 [R7]: Given `/old` answers 301 to `/new?utm_source=old`, `/new` with no query answers 301
  to `/new?utm_source=old`, and that URL answers a 200 page, when `rotguard --apply` runs on a
  link to `M/old`, then `post.md` links `M/new`.
- S23 [R8, R11]: Given `/old` answers 301 to `/new?utm_source=old` (200) and `/new` with no query
  answers 404, when `rotguard --apply` runs on a link to `M/old`, then the link is kept.
- S24 [R8]: Given `/old` answers 301 to `/new?utm_source=old` (200) and `/new` with no query
  answers 302 to `/elsewhere` (200), when `rotguard --apply` runs on a link to `M/old`, then the
  link is kept.
- S25 [R8]: Given `/old` answers 301 to `/new?utm_source=old`, a 200 page titled "Guide", and
  `/new` with no query answers a 200 page titled "Welcome", when `rotguard --apply` runs on a
  link to `M/old`, then the link is kept.
- S26 [R6, R8]: Given `/old` answers 301 to `/new?utm_source=old` (200) and `/new` with no query
  waits longer than `timeout_ms`, when `rotguard --apply` runs with a deadline longer than that
  wait, then the link is kept and M's log shows exactly 1 request for `/new` with no query.
- S27 [R14]: Given the S26 site and a `deadline_s` shorter than `timeout_ms`, when
  `rotguard --apply` runs, then the entry reads `UNVERIFIABLE deadline` and `post.md` is
  unchanged.
- S28 [R13]: Given the S19 site and M listed with `soft404 = false`, when `rotguard --apply`
  runs, then `post.md` links `M/us/getting-started`.
- S29 [R12]: Given every path under `/s/article/` answers 301 to itself plus `?language=en_US`,
  and each of those URLs answers the same 200 page titled "Page not found", when
  `rotguard --apply` runs on a link to `M/s/article/gone`, then the entry reads `SUSPECT` and
  `post.md` is unchanged.
- S30 [R11]: Given the S1 site, when `rotguard --strict --format json` runs, then `findings` is
  empty, `summary.ok` is 1, `summary.permanent` is 0 and the exit code is 0.
- S31 [R3, R10]: Given `/a` answers 301 to `/a?utm_source=x` (200), when `rotguard --apply` runs
  on a link to `M/a`, then the link is kept and M's log shows exactly 2 requests for `/a`.
- S32 [R6, R10]: Given `/old` answers 301 to `/new?ref=old` (200), when `rotguard --apply` runs
  on a link to `M/old`, then `post.md` links `M/new?ref=old` and M's log shows exactly 1 request
  for `/new`.
- S33 [R7]: Given the S19 site, when `rotguard --apply` runs on a link to `M/old#start`, then
  `post.md` links `M/us/getting-started#start`.

### Details
**New functions in `src/check.rs`**, all pure:

```text
fn locale_shaped(segment: &str) -> bool                    // R2
fn is_noise(from: &Url, to: &Url) -> bool                  // R1
fn without_tracking(fix: &Url, link: &Url) -> Option<Url>  // R6, R9; None when nothing to strip
const TRACKING_PREFIXES: &[&str] = &["utm_"];
```

`is_noise` compares `origin()`, then the decoded `query_pairs()` as multisets, then the paths.
Path segments come from `path_segments()` with 1 trailing empty segment dropped, so `/` is the
empty list and `/a/` equals `/a`. Segments are compared as the `url` crate serialises them, case
kept. The cases:

| U → V | Noise | Why |
|---|---|---|
| `/a` → `/a?x=1` | yes | pair added, same path |
| `/a?b=1&a=2` → `/a?a=2&b=1` | yes | same pairs, same path |
| `/a` → `/a#top` | yes | fragments ignored, same path |
| `/a` → `/a/?x=1` | yes | pair added, slash ignored |
| `/a` → `/a/` | no | only the slash changed |
| `/a?id=5` → `/a?id=6` | no | a pair of U is missing |
| `/n/x/` → `/en-gb/n/x` | yes | 1 locale segment inserted, slash ignored |
| `/n/x` → `/n/x/de` | yes | inserted at the end |
| `/` → `/en-gb/` | yes | inserted into the empty list |
| `/n/x` → `/en/gb/n/x` | no | 2 segments inserted |
| `/n/x` → `/en/n/y` | no | another segment changed |
| `/en/x` → `/en-US/x` | no | replaced, not inserted |
| `/help/en/a` → `/help/a` | no | removed |
| `/docs/x` → `/docs/Reference/x` | no | not locale-shaped |
| `http://m/a` → `https://m/a?x=1` | no | scheme differs |
| `/n/x?a=1` → `/de/n/x?a=1&l=de` | yes | locale inserted and pair added |

`locale_shaped` accepts `en`, `DE`, `en-gb`, `en_US`, `pt-BR`, `zh-Hans`, `es-419` and rejects
`eng`, `e`, `v2`, `en-`, `en-g`, `en-gbr`, `zh-Hans-CN`, `plans`, `Reference` and the empty
string. Unit tests in `src/check.rs` hold both tables.

**The fix choice in `classify`**, in the `200..=299` arm, with `leading = permanent_run(chain)`:

```text
clean = count of i in 0..leading, from 0, while !is_noise(chain[i].url, chain[i+1].url)
root rule (soft404 on, chain[0] not root):
    last is root, or leading > 0 and chain[leading] is root          // 0.2.1, unchanged
    or clean > 0 and chain[clean] is root                            // R5
        => Suspect "redirect to root"
leading > 0 and clean == 0  => Ok { chain }                          // R3
leading > 0                 => Permanent { chain, fix: chain[clean].url }   // R4
otherwise                   => as 0.2.1 (Temporary, Ok, unexpected redirect)
```

One existing unit case changes its expectation: in
`redirects_to_the_root_classify_by_soft404_and_final_status`, "a root url redirected to the
root is unaffected" feeds `[root 301, root 200]`, 2 identical URLs, which R1(a) makes a noise
step, so it now expects `Ok` instead of `Permanent(root)`. A real fetch never produces that
chain, because a repeated URL ends as `Loop`. "A middle root hop keeps the permanent fix"
(`/archives/123` → `/?p=123` → `/2020/post/`) has no noise step and keeps its fix.

`Verdict::Ok` keeps its `chain` field; nothing outside tests reads it. `no_location_reason`
keeps using `permanent_run` and is unchanged. The doc comment on `Verdict::Permanent.fix` and on
`classify` change to say where the fix points.

**The strip step in `check_one`.** It runs last, on a verdict that is still `Permanent` after
the soft-404 checks, and also on the early return for a host with `soft404 = false` (R13):

```text
if verdict is Permanent { chain, fix } and let Some(stripped) = without_tracking(&fix, url):
    match fetcher.fetch(&stripped):                       // 1 chain, no retry, paced
        Ok(f) if classify(&Ok(f), &HostConfig::default()) is Ok
                 and title_of(f.body) == title_of(fetched.body)
              => Permanent { chain, fix: stripped }       // R7
        _     => Ok { chain }                             // R8
```

`url` is the link as checked (fragment already stripped), so `without_tracking` compares keys
against the link's own query. It works on the raw query string: split on `&`, drop a piece when
the percent-decoded text before its first `=` starts with a `TRACKING_PREFIXES` entry (ASCII
case ignored) and equals no decoded key of the link's query, and join the rest with `&`. It
returns `None` when no piece is dropped, and calls `set_query(None)` when none is left.
`classify` with `HostConfig::default()` applies the root rule to the stripped URL's own chain,
so a stripped URL that redirects to the root is rejected. Two empty titles are equal.

The step sits inside the existing `timeout_at(deadline, check_one)`, which gives R14 with no new
code. `lib.rs` re-adds the link's fragment to the fix string as today.

**Tests.** A new `tests/acceptance/redirect_noise.rs`, registered in `main.rs`, holds S1 to S33
as `redirect_noise_sN_<description>`. Routes match by path, so a redirect that differs only by
query is 1 `Routes::with` handler that branches on `req.url.query()`. Request counts use
`w.log.hits(path)` filtered by `Req.query`; S19 counts `w.log.all()` filtered by
`path.starts_with("/us/")`, so a probe at `/us/<token>` shows. S26 and S27 follow the
hanging-server tests in `tests/acceptance/check.rs`. The mock's unmatched paths answer 404, so the soft-404 probe of a
kept link finds no evidence. S29 routes the whole folder, because a probe that answers 404
skips the title phrase check. That needs a regex route with a handler, which the fixture lacks:
add `Routes::with_re(pattern, handler)` to `tests/acceptance/support.rs`, beside `with` (exact
path, handler) and `fixed_re` (regex, fixed response).

**Docs.**
- README class table: `PERMANENT REDIRECT` reads "301/308 to a live page at another address; a
  redirect that only adds query parameters or a locale path segment (`/en-gb/`) is OK", with the
  fix "the redirect target, without `utm_*` parameters the redirect added".
- `docs/architecture.md`: Check flow step 5, a "Permanent fixes" paragraph after the soft-404
  section (noise steps, where the fix points, the strip and its proof), the first Decisions line
  ("can't be wrong") citing this design's D1 and D3, and a Hard parts entry for the noise rule.
- `docs/config.md`: `host_interval_ms` lists the stripped-URL request among the requests it
  counts.
- `CHANGELOG.md` `[Unreleased]`: 4 bullets. Kept links, saying a report-only run whose only
  findings were such redirects now exits 0. The fix that stops before a noise step (`/old` →
  `/new` → `/new?language=en_US` is rewritten to `/new`). The `utm_*` strip, naming the request
  it may add. A link whose fix would now be the site root (`/promo` → `/` → `/en-gb/`) is
  `SUSPECT redirect to root` instead of rewritten, and gets a snapshot lookup like any suspect.
- `docs/backlog.md`: remove the 2 `[idea]` lines this design covers (the `utm_*` line from shape
  404-walls and the "only localises" line from 2026-11-11).

- **Facts**:
  - probed: rotguard 0.2.1 release build, report-only on the affilytics blog from a UK machine:
    241 links, 11 permanent redirects. A script applying R1 to R6 to their chains keeps 2
    (success.awin.com, newsroom.pinterest.com), strips 1 (www.shareasale.com) and gives the
    other 8 the fix 0.2.1 gives: MDN, linktr.ee (1 hop, 308, `/en/` removed), prettylinks,
    hls.harvard.edu and amzwatcher (trailing slash added), developers.google.com, beehiiv (308,
    308, 200) and support.squarespace.com (slug changed).
  - probed: same build on a scratch site, `https://linktr.ee/help/en/articles/8619620` answers
    308, 308, 200, the chain S8 copies.
  - probed: curl, newsroom.pinterest.com answers `/en-gb/news/…` with no `Accept-Language`,
    with `en-US` and with `zz`, `/de/news/…` with `de` and `/fr/news/…` with `fr`. Its 301
    carries `vary: User-Agent, Accept-Encoding` only.
  - probed: curl, the 301/308 targets of the MDN and linktr.ee links are the same with and
    without `Accept-Language: de`; MDN answers its locale-less URL with a 302.
  - probed: curl with rotguard's User-Agent, `https://www.awin.com/us/getting-started-sas`
    answers 200 with no redirect, the same `<title>` as the URL with the 2 `utm_*` parameters,
    and the same visible text (3-word shingle similarity 1.0).
  - probed: curl, support.zendesk.com `/hc/articles/4408846795674` answers 301 to
    `/hc/en-us/articles/…` (`/hc/de/…` with `Accept-Language: de`), then 301 to the URL with a
    slug.
- **Invariants**:
  - A fix rotguard writes for a permanent redirect is a URL its own chain reached, or a
    stripped URL that R7 proved; owner: `classify` and `check_one`.
  - For a link whose path is not `/`, on a host with soft-404 detection on, a fix R4 stops
    before a noise step, and a stripped URL R7 accepts, classify `OK` on the next run while the
    site answers the same; owner: `classify`. A fix at the end of a run that
    a 302, 303 or 307 follows is `TEMPORARY REDIRECT` on the next run, as in 0.2.1.
  - The noise rule and the strip never add a write. They turn a `Permanent` into `Ok`, move its
    fix to an earlier hop or a stripped URL, make it `Suspect` under R5, or leave it
    `Unverifiable` under R14; they never make a link `Dead` or `Permanent`; owner: `classify`
    and `check_one`.
  - The strip fetch runs through `Fetcher::fetch` inside the per-URL `timeout_at(deadline)`;
    owner: `check_all`.
- **Budgets**: the strip adds, per link whose fix has added `utm_*`, 1 fetch of the stripped
  URL: 1 GET, or up to 11 when it redirects 10 times, with no retry. Each GET queues at its
  host's gate behind that host's other requests (starts are at least `host_interval_ms` apart,
  default 1000, and a slow request holds the gate), then runs for at most `timeout_ms` (default
  10 s). The per-URL `timeout_at(deadline)` bounds the whole wait.
  Measured on the awin target: 1 GET of about 0.1 s. The noise rule adds 0 requests to linked
  sites; a link R5 makes `SUSPECT` gets the snapshot lookup every suspect gets, which 0.2.1
  didn't send for it.

### Risk paths
As in `.whittle/gates.toml`; this design touches none of them.

### Fitness rules
As in `.whittle/gates.toml`: only `src/apply.rs` writes files, tests never hit the real network,
no private data in tracked files.

### Review changes
- Round 1: R7 says the stripped URL is judged by `classify` alone (root rule on, no soft-404
  probe), matching Details and the 1-request budget.
- Round 1: D3 no longer claims `utm_*` never selects content; its cost names personalisation
  under an unchanged title.
- Round 1: the Summary, "Where the fix points", the Invariants and the CHANGELOG bullets say
  what changes from 0.2.1 besides kept links: the fix before a noise step (S16), `SUSPECT
  redirect to root` under R5 (S18) and `UNVERIFIABLE deadline` under R14.
- Round 1: the next-run invariant covers only fixes R4 stops before a noise step and stripped
  URLs; a fix followed by a 302 stays `TEMPORARY REDIRECT` next run.
- Round 1: the overview's strip proof reads "through a redirect whose first step is noise",
  matching R3 and R7.
- Round 1: the strip's cost and budget say the request queues at its host's gate, bounded by
  the deadline, instead of "up to `host_interval_ms`".
- Round 1: Details names `Routes::with_re` in `tests/acceptance/support.rs`, which S29 needs.
- Round 1: Details says the unit case "a root url redirected to the root is unaffected" now
  expects `Ok`.
- Round 2: R10 and the budget count requests to linked sites only, and say a link R5 makes
  `SUSPECT` gets a snapshot lookup; the CHANGELOG bullet names it.
- Round 2: the next-run invariant holds for links whose path is not `/`.
- Round 2: S19 counts every request under `/us/`, so a soft-404 probe of the stripped URL fails
  it.
- Round 3: the next-run invariant holds for links on a host with soft-404 detection on.
- Round 3: Details says how S19 counts requests under `/us/` (`w.log.all()` by path prefix).
- Round 3: the budget no longer puts a number on the snapshot lookup a suspect gets.
