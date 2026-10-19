# Backlog

- [idea] Weekly GitHub Action that runs `rotguard --apply` on the site and opens a PR with the fixes — from shape link-health, 2026-10-19
- [idea] Run-state file so a link counts as dead only after N consecutive failing runs and flaky hosts stop flapping in CI — from shape link-health, 2026-10-19
- [P2] Say whether a DNS failure gets the connection-error retries before the dead re-check — Design, Inside check.rs — from shape link-health, 2026-10-19
- [P2] State that run appends the link's fragment to the fix when building each Finding (R8, R16) — Design, Data model — from shape link-health, 2026-10-19
- [P2] Give Rust shapes for Finding and apply's per-file outcome — Design, Data model / Apply — from shape link-health, 2026-10-19
- [P2] R23: add unwritable-fix rules for `Ctx::MdText` (`'`, trailing `.,;:!?)`) and `Ctx::Html` (record the attribute quote in `Ctx`) — Requirements R23, Extract — from shape link-health, 2026-10-19
- [P2] `site_url`/`archive_url` accept non-http schemes and host-less URLs; require http(s) and a host — src/config.rs:94 — from whittle/link-health, 2026-10-19
- [P2] `deadline_s` is unbounded, so `Instant + deadline` can overflow and panic; bound it or use `checked_add` — src/config.rs:116 — from whittle/link-health, 2026-10-19
- [P2] `ConfigError` is public but not `#[non_exhaustive]` — src/config.rs:20 — from whittle/link-health, 2026-10-19
- [P2] `ConfigError` prints its source in `Display` and also returns it from `source()`, so chain reporters print it twice — src/config.rs:35 — from whittle/link-health, 2026-10-19
- [P2] `[hosts]` keys keep their case, but `Url::host_str()` is lowercase, so `[hosts."GoodReads.com"]` never matches (R13); lowercase the keys — src/config.rs:118 — from whittle/link-health, 2026-10-19
- [P2] A YAML value that is only an anchor or tag (`a: &x`, `a: !foo`) makes the lexer skip the nested block, so its URLs fall back to non-verbatim whole-block spans — src/extract.rs:421 — from whittle/link-health, 2026-10-19
