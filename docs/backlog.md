# Backlog

- [idea] Run-state file so a link counts as dead only after N consecutive failing runs and flaky hosts stop flapping in CI — from shape link-health, 2026-10-19. Held off 2026-11-08: 0 dead links in the blog's 6 runs (2026-10-20 to 11-02); all flaky links were UNVERIFIABLE, which never fail the job. Revisit when a dead link flaps
- [idea] Raise a link that stays UNVERIFIABLE for N runs in a row as needing a look (langsec.org failed TLS in all 6 blog runs, visible only in the folded list) — from shape dead-streak, 2026-11-08
- [idea] Second soft-404 probe that also replaces the ID segment, so hosts that ignore the slug and serve the article at a made-up URL without a redirect (support.google.com threads, advertising.amazon.com partner pages) stop showing as SUSPECT; a month-archive soft-404 (`/2020/05/<slug>`) would fool it — from shape foreign-sites, 2026-11-08
- [idea] One CDX query per URL shared across page dates (the affilytics run made 15 lookups for 6 dead URLs, 13 of them for 3 ftc.gov URLs); dead-first ordering made it unnecessary for now — from shape foreign-sites, 2026-11-08
