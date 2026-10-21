---
status: approved
---
# A redirect through the root is suspect only when the link ends there

## Overview

### Problem
rotguard treats a link that redirects to a site's home page as a removed page: it reports
`SUSPECT redirect to root` and proposes an archived copy, since sites often send dead URLs home.
But today any hop through `/` counts, not only where the chain ends. A WordPress permalink such
as `/archives/123` often redirects to `/?p=123` and then to `/2020/post/`, a live page. rotguard
reports that link as suspect, so it gets no permanent fix and the weekly job fails until someone
fixes it by hand. The `?p=123` hop has the path `/`, and that is all the check looks at. None of
the 100 external links on your site look like this today, so this fixes a wrong verdict before a
link hits it.

### Use cases
**A WordPress permalink that moved.** `M/archives/123` answers 301 to `M/?p=123`, which answers
301 to `M/2020/post/`, a 200 page. rotguard reports
`PERMANENT REDIRECT 301 301 200` with `fix: M/2020/post/`, and `--apply` writes that URL. Today
it reports `SUSPECT redirect to root`.

**A link sent home is still caught.** `M/old` answers 301 to `M/new`, which answers 301 to `M/`.
The chain ends at the home page, so rotguard reports `SUSPECT redirect to root`, as today.

**A permanent fix never points at the home page.** `M/a` answers 301 to `M/`, which answers 302
to `M/b`. The permanent part of the chain ends at `M/`, so a fix would rewrite the link to the home
page. rotguard reports `SUSPECT redirect to root` instead and writes nothing. Today it does the
same, because `/` is a hop.

### How it works
`classify` in `src/check.rs` turns a fetch's redirect chain into a verdict. Only the
redirect-to-root test changes, and only for a chain that ends in a 2xx answer. It now looks at
two URLs: where the link ends up (the last hop), and where its permanent fix would point (the end
of the leading 301/308 run). Any other chain never reached a page: it failed (a redirect loop, a
redirect with no `Location`, too many hops) or ended in a status such as 400 or 405. For those,
any hop through `/` still makes the link suspect.

```text
Context: one link check (src/check.rs)

  fetch(url) ──chain of hops──► classify(outcome, host)
                    │ 404/410 → Dead; 401/403/429/999/5xx → Unverifiable  (unchanged)
                    │
                    │ other status or failed: any hop is "/"    → Suspect  (unchanged)
                    │ 2xx: last hop's path is "/"               → Suspect  (unchanged)
                    │ 2xx: leading 301/308 run ends at "/"      → Suspect  (new)
                    │ 2xx: a middle hop is "/", ends elsewhere  → Permanent / Temporary
                    │                                              (new; was Suspect)
                    ▼
                 Verdict ──► report, --apply
```

**Main flow, the WordPress permalink.**
1. The fetch follows `/archives/123` (301) → `/?p=123` (301) → `/2020/post/` (200).
2. The last hop's path is `/2020/post/`, not `/`, so the chain doesn't end at the root.
3. The leading 301/308 run is two hops long and ends at `/2020/post/`, which isn't `/` either.
4. So the verdict is `Permanent` with fix `/2020/post/`, and the soft-404 checks run on that page
   as they do for any 2xx answer.

**Failure flow.** A chain whose permanent run ends at `/` and then goes on (`/a` 301 → `/` 302 →
`/b`) is suspect, so `--apply` never rewrites a link to the home page. With `soft404 = false` for a
host, none of this applies, as today.

**Contract.**

```text
classify(outcome, host) -> Verdict                         // signature unchanged
  failed, link path != "/", any later hop path == "/"       → Suspect "redirect to root"
  non-2xx, link path != "/", any later hop path == "/"      → Suspect "redirect to root"
  2xx, link path != "/", last hop path == "/"               → Suspect "redirect to root"
  2xx, link path != "/", leading > 0, chain[leading] == "/" → Suspect "redirect to root"
  host.soft404 == false                                     → none of the above
```

### Decisions
#### D1: Only where the link ends up, or where its fix would point, counts (assumption)
Options: keep "any hop through `/`"; skip hops whose URL has a query (`/?p=123`); look only at the
last hop and the fix target. Chosen: the last. Why: a removed page sends the reader home and stops
there, while a chain that goes on to another page reached a real page, whose soft-404 checks still
run. Skipping query hops would still flag `/a` → `/` → `/b`. Checking the fix target keeps the
one wrong write the old rule prevented: rewriting a link to the home page. Cost if wrong: low; a
site that bounces dead URLs through `/` to a page that is neither a not-found title nor the same as
its probe would get a permanent fix to that page.

#### D2: Chains that never reach a page keep the old rule (assumption)
A loop, a missing `Location` or too many hops never reach a page, and neither does a chain that
ends in a status such as 400 or 405. There is no end page to judge, so a visit to `/` on the way
is still the best sign the link was removed. Keeping the rule means these verdicts don't change:
`/a` 301 → `/` 301 → `/x` 400 stays suspect instead of becoming `UNVERIFIABLE 400`.

#### D3: A final URL `/` with a query still counts (assumption)
`/old` → `/?s=old` (200) ends at the home page or its search, so it stays suspect. A site that
serves posts at `/?p=123` itself would be flagged; that is the same verdict as today.

#### D4: This settles the backlog's R10 line
The `[idea]` "Decide whether R10 means any hop through `/` or only the final URL" is settled by
D1 and leaves the backlog.

### Scope
In:
- The redirect-to-root test in `classify`, ~10 lines in `src/check.rs`: any WordPress-style link
  you add later. Your site has none today.
- Docs: `docs/architecture.md`'s soft-404 sentence, ~2 lines.

Out: changing what other soft-404 checks see (titles, probes); query strings in any rule; hosts with
`soft404 = false`; chains that don't end in a 2xx answer (D2); a per-host option for this.

Size: ~12 non-test lines in total.
Touches: none.

### Open for you
none

## Spec

### Requirements
- R1: WHEN a fetch of a URL whose path isn't `/` ends in a 2xx answer and its last hop's path is
  `/` THE SYSTEM SHALL classify it suspect with the reason `redirect to root`, whatever hops come
  before.
- R2: WHEN a fetch of a URL whose path isn't `/` ends in a 2xx answer at a path other than `/`,
  and its leading 301/308 run doesn't end at `/`, THE SYSTEM SHALL classify it by its statuses
  (permanent, temporary) even if a middle hop's path is `/`.
- R3: IF a fetch of a URL whose path isn't `/` ends in a 2xx answer and its leading 301/308 run
  ends at a URL whose path is `/` THEN THE SYSTEM SHALL classify it suspect with the reason
  `redirect to root` and propose no permanent fix.
- R4: WHEN a fetch of a URL whose path isn't `/` fails as a redirect loop, a redirect without a
  `Location` or too many redirects, or ends in a status that isn't 2xx and isn't dead (404, 410)
  or unverifiable (401, 403, 429, 999, 5xx), and any hop after the first has the path `/`, THE
  SYSTEM SHALL classify it suspect with the reason `redirect to root`, as today.

### Acceptance scenarios
M is the mock server's base URL. Mock routes match the path only, so `M/?p=123` is served by the
route for `/`. Unmatched paths answer 404.

- S1 [R2]: Given `M/archives/123` answering 301 to `M/?p=123`, `M/` answering 301 to
  `M/2020/post/`, and `M/2020/post/` answering 200 with title "Post" and the body
  `first post text here`, when rotguard runs, then the entry for `M/archives/123` reads
  `PERMANENT REDIRECT 301 301 200`, its detail line is `    fix: M/2020/post/`, no entry reads
  `SUSPECT`, and the exit code is 1.
- S2 [R3]: Given `M/a` answering 301 to `M/`, `M/` answering 302 to `M/b`, and `M/b` answering 200
  with title "B" and the body `page b words`, when rotguard runs with `--apply`, then the entry for
  `M/a` reads `SUSPECT redirect to root`, stdout contains no `PERMANENT REDIRECT`, and `post.md`
  is unchanged.
- S3 [R1]: Given `M/old` answering 301 to `M/new`, `M/new` answering 301 to `M/`, and `M/`
  answering 200 with title "Home" and the body `welcome`, when rotguard runs, then the entry for
  `M/old` reads `SUSPECT redirect to root` and stdout contains no `PERMANENT REDIRECT`.
- S4 [R4]: Given `M/deep/loop` answering 301 to `M/` and `M/` answering 301 to `M/deep/loop`, when
  rotguard runs, then the entry for `M/deep/loop` reads `SUSPECT redirect to root`.

### Details
- `classify` (src/check.rs:313) calls `redirects_to_root` in two places. The failed-chain arm
  (:322-331) keeps today's any-hop test. The `_ if redirects_to_root(chain, host)` arm in
  `match last.status` (:343) keeps the any-hop test but no longer matches a 2xx status. In the
  `200..=299` arm, with `host.soft404` on and the link's path not `/`, a last hop whose path is
  `/`, or (when `leading > 0`) a fix whose path is `/`, gives
  `Suspect { reason: "redirect to root" }` before the Permanent, Temporary and Ok branches. Every
  rule still requires the first hop's path to differ from `/`.
- The unit table `redirects_to_the_root_classify_by_soft404_and_final_status` (src/check.rs:1273)
  has no chain with a middle `/` hop, so none of its 13 cases change. It gains rows for
  `/archives/123` 301 → `/?p=123` 301 → `/2020/post/` 200 (Permanent), `/a` 301 → `/` 302 → `/b`
  200 (Suspect), `/a` 302 → `/` 301 → `/b` 200 (Temporary), and `/a` 301 → `/` 301 → `/x` 400
  (Suspect, as today).
- The existing acceptance tests S15 and S43 in tests/acceptance/check.rs end at `/` and keep
  passing.
- `docs/architecture.md:80-81` says "when the page redirected a non-root URL to the site root";
  it becomes: when a non-root link ends at the site root, or its permanent fix would point there.
- The backlog's R10 line cites `src/check.rs:284`; it is deleted at approval (D4).

### Risk paths
none touched; `.whittle/gates.toml` unchanged.

### Fitness rules
Unchanged: only `src/apply.rs` writes files; tests use no real-network URLs;
`scripts/privacy-check.sh --all`.

### Review changes
- Round 1: a chain that passes through `/` and ends in a non-2xx status such as 400 keeps
  today's any-hop rule (D2, R4), so it stays suspect instead of becoming `UNVERIFIABLE 400`.
