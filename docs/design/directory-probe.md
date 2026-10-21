---
status: approved
---
# Probe a sibling for a link that ends in `/`

## Overview

### Problem
To spot a soft 404, rotguard asks the same site for a made-up page next to the link. When that
page looks the same as the link, the link is probably a "not found" page served with a 200. The
made-up page is built by replacing the link's last path segment with a random 24-character
token. But for a link that ends in `/`, such as `https://ziglang.org/learn/tools/`, the last
segment is empty. So today the probe is `/learn/tools/<token>`, a page inside the link instead of
next to it. A site that answers unknown sub-paths with the section's own page then sends back
the very page the link points to. rotguard calls the live link `SUSPECT` (`probe: same title`),
and an unfixed suspect makes the weekly job fail. 12 of your roughly 100 external links end in a
path with `/`. All of them pass today, but each new docs or blog link of this shape takes the risk.

### Use cases
**A section page whose site answers every sub-path with it.** `M/docs/` is a docs home titled
"Docs home", and the site answers any `M/docs/<anything>` with that same page. Today rotguard
probes `M/docs/<token>`, gets "Docs home", and reports `SUSPECT probe: same title`. After this
change it probes `M/<token>/`, which answers 404, and the link is ok.

**A dead directory-style link is still caught.** `M/blog/gone/` was removed, and the blog now
answers every unknown `M/blog/<anything>/` with its index page, titled "Blog", and a 200.
rotguard probes `M/blog/<token>/`, gets the same title, and reports
`SUSPECT probe: same title`, as it would for `M/blog/gone` without the slash.

### How it works
`src/check.rs` builds the probe in `Probes::probe`, shared by the soft-404 check and the bot-wall
check. Only the probe's path and its cache key change. The comparisons after the probe answers
stay as they are.

```text
Context: one link check (src/check.rs)

  link URL ──2xx, or 401/403/999──► Probes::probe(url)
                                       │ path ends in "/" ?
                                       ├─ no:  /a/b     → /a/<token>    (unchanged)
                                       └─ yes: /a/b/    → /a/<token>/   (new; was /a/b/<token>)
                                       │
                                       ▼ one fetch per (origin, parent, ends in "/")
                                    ProbeResult ──► soft-404 reason / bot-wall verdict (unchanged)
```

**Main flow, a directory link.**
1. `/learn/tools/` answers 200, so rotguard asks for its probe.
2. The path ends in `/`, so the probe drops the trailing slash, replaces the last segment
   (`tools`) with the token, and puts the slash back: `/learn/<token>/`.
3. The probe's result is cached under (origin, `/learn`, ends in `/`). Every other directory link
   under `/learn/` reuses it. A link without the slash, such as `/learn/notes`, gets its own probe
   `/learn/<token>`.
4. The link's page is compared with the probe's answer, as today.

**Failure flow.** A probe that can't be fetched gives no soft-404 reason, as today. A bot wall on
a directory link (`/docs/` answers 403) is now decided by the sibling probe `/<token>/`.

**Contract.**

```text
Probes::probe(&self, fetcher, url) -> Arc<ProbeResult>      // signature unchanged
  path "/a/b"  → probe "/a/<token>",  cache key (origin, "/a", false)
  path "/a/b/" → probe "/a/<token>/", cache key (origin, "/a", true)
  path "/docs/" → probe "/<token>/",   cache key (origin, "",   true)
  path "/"     → no soft-404 probe (unchanged)
```

### Decisions
#### D1: Replace the last non-empty segment and keep the slash (assumption)
Options: skip the probe for links that end in `/` and rely on not-found titles alone; probe a
sibling without the slash (`/a/<token>`); probe a sibling with the slash (`/a/<token>/`). Chosen:
a sibling with the slash. Why: R11 already means "next to the link", and a probe shaped like the
link takes the same route through the site's server (many sites redirect `/a/x` to `/a/x/`).
Skipping would lose soft-404 detection for directory links. Cost if wrong: low; a site that
answers every top-level `/<anything>/` with one page would still fool the probe for a link like
`/docs/`, and `soft404 = false` for that host covers it, as it does today.

#### D2: The slash is part of the cache key (assumption)
`/learn/a/` and `/learn/c` now probe different URLs under the same parent, so the cache key is
(origin, parent path, ends in `/`). This adds at most one probe per parent path that has both
shapes of link. Your site has no such parent today.

#### D3: This settles the backlog's R11 line
The `[idea]` "Decide R11's probe for a path ending in `/`" is settled by D1 and leaves the
backlog.

### Scope
In:
- The probe path and cache key in `Probes::probe`, ~10 lines in `src/check.rs`: the 12 links on
  your site that end in a path with `/`.
- Docs: `docs/architecture.md`'s soft-404 paragraph, ~2 lines.

Out: changing what counts as "the same page" (final URL, title, body similarity); the root path
`/`, which still has no soft-404 probe; links with a query string, whose probe already drops the
query; R10 (a redirect through `/`), which stays in the backlog; a probe at more than one level.

Size: ~12 non-test lines in total.
Touches: none.

### Open for you
none

## Spec

### Requirements
- R1: WHEN rotguard probes a URL whose path ends in `/` and isn't `/` THE SYSTEM SHALL probe the
  path with its last non-empty segment replaced by the 24-character token and the trailing `/`
  kept (`/a/b/` → `/a/<token>/`, `/docs/` → `/<token>/`).
- R2: WHEN rotguard probes a URL whose path doesn't end in `/` THE SYSTEM SHALL probe the path
  with its last segment replaced by the token, as today (`/docs/real` → `/docs/<token>`).
- R3: THE SYSTEM SHALL fetch one probe per (origin, parent path, whether the path ends in `/`)
  per run, and use it for both the soft-404 check and the bot-wall check.

### Acceptance scenarios
M is the mock server's base URL. A probe path is one whose token is 24 characters of `[a-z0-9]`.
Unmatched paths answer 404.

- S1 [R1]: Given `M/docs/` answering 200 with title "Docs home" and the body
  `alpha beta gamma delta epsilon`, and every `M/docs/<token>` (regex
  `^/docs/[a-z0-9]{24}$`) answering 200 with the same title and body, when rotguard runs, then
  `/docs/` is not reported, the exit code is 0, and `M` received exactly one probe, whose path
  matches `^/[a-z0-9]{24}/$`.
- S2 [R1]: Given `M/blog/gone/` answering 200 with title "Blog" and the body
  `latest posts list here`, and every `M/blog/<token>/` (regex `^/blog/[a-z0-9]{24}/$`)
  answering 200 with title "Blog" and the body `one two three four five six`, when rotguard
  runs, then the entry for `M/blog/gone/` reads `SUSPECT probe: same title` and `M` received
  exactly one probe, whose path matches `^/blog/[a-z0-9]{24}/$`.
- S3 [R2, R3]: Given `M/learn/a/`, `M/learn/b/` and `M/learn/c` each answering 200 with a
  different title and body, and every probe answering 404, when rotguard runs, then none of the
  three is reported, the exit code is 0, and `M` received exactly two probes: one whose path
  matches `^/learn/[a-z0-9]{24}/$` and one whose path matches `^/learn/[a-z0-9]{24}$`.
- S4 [R1, R3]: Given `M/docs/` answering 403 and every `M/<token>/` (regex
  `^/[a-z0-9]{24}/$`) answering 403, when rotguard runs with `--strict`, then the entry for
  `M/docs/` is `BLOCKED 403`.

### Details
- In `Probes::probe`, take the path, note whether it ends in `/` (and isn't `/`), and strip that
  one trailing slash before `rsplit_once('/')` picks the parent, as today. Build the probe as
  `{parent}/{token}` plus `/` when the slash was stripped. The cache key gains the bool.
- `Probes::reason` already skips the probe for the path `/`; the bot-wall check in `check_one`
  calls `probe` on the final URL of a 401/403/999 and keeps doing so.
- The acceptance helper `Log::probes()` (tests/acceptance/support.rs:76) and `is_probe` look at
  the last path segment, which is empty for `/<token>/`, so they don't see the new probes. The
  new tests count probes from `w.log.all()` with the scenario's regex instead; the locked support
  file doesn't change. No existing acceptance test links a non-root path that ends in `/` with
  soft-404 on, so none of their request counts change.
- `docs/architecture.md:80-83` says the probe has "a random last segment under the same parent
  path"; it gains that a link ending in `/` is probed as a sibling folder, `/a/<token>/`.

### Risk paths
none touched; `.whittle/gates.toml` unchanged.

### Fitness rules
Unchanged: only `src/apply.rs` writes files; tests use no real-network URLs;
`scripts/privacy-check.sh --all`.

### Review changes
- Round 1: builder check only; it found nothing blocking (five P3 wording notes, dropped).
