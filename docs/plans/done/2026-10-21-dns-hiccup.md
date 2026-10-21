# Plan: DNS hiccup isn't dead
Design: docs/design/dns-hiccup.md
Risk paths: none
Diff estimate: 45

### T1: Split error_kind out of map_err
- Files: src/check.rs
- Interfaces: Produces `pub fn error_kind(messages: &[String]) -> FetchError` (messages
  lowercased, the error without its URL first, then each source), with today's behaviour exactly:
  any of the five DNS needles gives `Dns`, then the TLS words give `Tls`, else `Connect`.
  `map_err` keeps its timeout check, builds the messages and calls it.
- Test: the existing unit tests and acceptance tests stay green (a pure refactor, so the
  acceptance tests of T2 can compile against the library).
- Covers: none
- Review: skip
- Commit: Split error_kind out of map_err

### T2: Acceptance tests
- Files: tests/acceptance/dns.rs (new), tests/acceptance/main.rs
- Interfaces: Consumes `rotguard::check::{error_kind, classify, FetchError, Verdict}` and
  `rotguard::config::HostConfig` (T1).
- Test: one test per scenario S1–S4, named `dns_s<n>_…`, driving the library. S2 and S4 must not
  name the new variant, which doesn't exist yet: S2 asserts `error_kind(..).to_string() ==
  "dns lookup failed"` for each list, and S4 asserts that
  `classify(&Err(error_kind(&["dns error".into()])), &HostConfig::default())` is
  `Verdict::Unverifiable { reason: "dns lookup failed".into() }`. Both are marked
  `#[ignore = "pending: S2"]` / `"pending: S4"` and fail today. S1 and S3 pin behaviour the design
  keeps (R1, R5), so they pass from the start and carry no marker. `main.rs` is locked: open it
  with `whittle-gates amend` and add `mod dns;` only.
- Covers: S1, S2, S3, S4
- Commit: Add acceptance tests for DNS lookup failures

### T3: Treat a failed DNS lookup as unverifiable
- Files: src/check.rs
- Interfaces: Consumes `error_kind` (T1). Produces `FetchError::DnsFailed` (Display
  `dns lookup failed`); `error_kind` gives `Dns` only for the four R1 phrases, `DnsFailed` for the
  other DNS messages (R2), then `Tls`, then `Connect`; `retry_for` answers `Retry::Transient` for
  `DnsFailed`.
- Test: S5 as a unit test of `retry_for`. Add `DnsFailed` to `errors_display_their_reason` and a
  `DnsFailed` row to the `classify` table test. Un-pends S2 and S4.
- Covers: R1, R2, R3, R4, R5, S2, S4, S5
- Commit: Treat a failed DNS lookup as unverifiable

### T4: Docs
- Files: README.md, docs/architecture.md, docs/config.md
- Interfaces: none
- Test: none. Read the branch's diff against `Base:` in .whittle/run/progress.md.
  - README.md class table (~line 77-80): `DEAD` says "404, 410 or a host DNS says doesn't exist,
    on two tries"; `UNVERIFIABLE` adds "a DNS lookup that failed".
  - docs/architecture.md:27-29: "a missing host on two tries" becomes a host DNS says doesn't
    exist; a failed lookup is `UNVERIFIABLE`. Add a Decisions line citing
    `docs/design/dns-hiccup.md` D1.
  - docs/config.md:81 (`recheck_delay_ms`): "404, 410 or no DNS answer" becomes "404, 410 or a
    host DNS says doesn't exist".
- Covers: none
- Review: skip
- Commit: Update docs for DNS lookup failures

## Verification contract
```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
! grep -rnE 'fs::write|File::create|fs::rename|OpenOptions' src --include='*.rs' | grep -v '^src/apply.rs:'
! grep -rhoE 'https?://[A-Za-z0-9.-]+' tests | grep -vE '^https?://(127\.0\.0\.1|localhost|[A-Za-z0-9.-]+\.(invalid|test|example))$'
scripts/privacy-check.sh --all
```
