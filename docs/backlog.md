# Backlog

- [idea] Weekly GitHub Action that runs `rotguard --apply` on the site and opens a PR with the fixes — from shape link-health, 2026-10-19
- [idea] Run-state file so a link counts as dead only after N consecutive failing runs and flaky hosts stop flapping in CI — from shape link-health, 2026-10-19
- [idea] Decide whether a temporary resolver failure (EAI_AGAIN, "temporary failure in name resolution") is DEAD, as R7 makes every DNS failure today, or unverifiable like a connection error — src/check.rs:300 — from whittle/link-health, re-tagged 2026-10-20
- [idea] Decide whether R10 means any hop through `/` or only the final URL: today `/archives/123` → `/?p=123` → `/2020/post/` is SUSPECT and loses its permanent fix — src/check.rs:284 — from whittle/link-health, re-tagged 2026-10-20
- [idea] Decide R11's probe for a path ending in `/`: today `/docs/` probes the child `/docs/<token>`, not a sibling, so sites that send unknown sub-paths to the section index flag directory-style pages — src/check.rs:516 — from whittle/link-health, re-tagged 2026-10-20
- [idea] Decide R19's detail line for a fix `--apply` could not write: today it prints `fix: <url>` like an applied one, and only stderr and exit 2 differ — src/report.rs:95 — from whittle/backlog-2026-10-20, re-tagged 2026-10-20
- [idea] A `--quiet` (or `--progress`) flag to silence or force progress output on stderr — from shape progress-and-config, 2026-10-20
