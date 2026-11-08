# Backlog

- [idea] Run-state file so a link counts as dead only after N consecutive failing runs and flaky hosts stop flapping in CI — from shape link-health, 2026-10-19. Held off 2026-11-08: 0 dead links in the blog's 6 runs (2026-10-20 to 11-02); all flaky links were UNVERIFIABLE, which never fail the job. Revisit when a dead link flaps
- [idea] Raise a link that stays UNVERIFIABLE for N runs in a row as needing a look (langsec.org failed TLS in all 6 blog runs, visible only in the folded list) — from shape dead-streak, 2026-11-08
