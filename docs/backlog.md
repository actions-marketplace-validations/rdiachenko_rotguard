# Backlog

- [idea] Run-state file so a link counts as dead only after N consecutive failing runs and flaky hosts stop flapping in CI — from shape link-health, 2026-10-19
- [idea] Decide whether R10 means any hop through `/` or only the final URL: today `/archives/123` → `/?p=123` → `/2020/post/` is SUSPECT and loses its permanent fix — src/check.rs:284 — from whittle/link-health, re-tagged 2026-10-20
- [idea] Decide R11's probe for a path ending in `/`: today `/docs/` probes the child `/docs/<token>`, not a sibling, so sites that send unknown sub-paths to the section index flag directory-style pages — src/check.rs:516 — from whittle/link-health, re-tagged 2026-10-20
- [idea] A `--quiet` (or `--progress`) flag to silence or force progress output on stderr — from shape progress-and-config, 2026-10-20
- [idea] When rotguard goes public: a tag-triggered release workflow that publishes a Linux binary, and the action downloads it for its own tag instead of building (~1m 49s a run), so the site pins `@v0.1.0` or `@v0` instead of `@main` — from shape weekly-fix-pr, 2026-10-20
