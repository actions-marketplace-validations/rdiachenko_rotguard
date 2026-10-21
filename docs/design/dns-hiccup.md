---
status: approved
---
# DNS hiccup isn't dead: only a "no such name" answer makes a link dead

## Overview

### Problem
rotguard calls a link dead with reason `dns` whenever the host's name lookup fails twice, 5 s
apart. That covers a domain that is gone, but also a lookup that failed for a moment: the runner's
resolver timing out or answering "temporary failure in name resolution". The weekly action then
proposes a Wayback copy for that link, writes it with `--apply`, and opens a PR that swaps a live
link for an archive one. It is the one wrong write the unattended run can make on a healthy site.
No run has hit it yet (the two runs so far had 0 dead links), but CI resolvers fail this way more
often than a home connection does.

### Use cases
**A domain is gone.** `https://nonexistent.invalid/page` gets "no such name" from DNS. The report
still says `DEAD dns`, rotguard proposes an archived copy, and `--apply` writes it, as today.

**The runner's resolver hiccups.** `https://www.example.test/post` fails with
`dns error: failed to lookup address information: Temporary failure in name resolution`. rotguard
retries it like a refused connection, and if it still fails the report says
`UNVERIFIABLE dns lookup failed`. No fix is proposed, `--apply` leaves the link alone, and the
weekly job passes (unverifiable links fail a run only with `--strict`). In the markdown report it
sits in the folded list as `could not check (dns lookup failed)`.

### How it works
`src/check.rs` turns a fetch error into a `FetchError` kind by reading the error's messages
(`map_err`, the "dns error" branch). Today any DNS message gives `FetchError::Dns`, which
`classify` makes dead. This change splits that branch:

```text
fetch error messages (reqwest, getaddrinfo)
        │
        ▼
  error_kind(messages) ──"no such name / no address" phrase──► FetchError::Dns ──► DEAD dns
        │
        ├─ other DNS message ──► FetchError::DnsFailed ──► retried as transient
        │                                        └─ still failing ──► UNVERIFIABLE dns lookup failed
        ├─ tls, certificate, ssl, handshake ──► FetchError::Tls (unchanged)
        └─ anything else ──► FetchError::Connect (unchanged)
```

**Main flow, a hiccup.** The first fetch fails with a temporary DNS message, so its kind is
`DnsFailed`. `fetch_with_retries` retries it as it does a timeout or a refused connection. If a
retry succeeds, the link is checked as usual. If every retry fails, `classify` gives
`Unverifiable { reason: "dns lookup failed" }`, and the run goes on: no archive lookup, no fix.

**Failure flow, a gone domain.** The messages contain a definite answer such as
`Name or service not known`, so the kind is `Dns` and nothing changes: dead, rechecked after
`recheck_delay_ms`, archive lookup, fix.

**Contract.**

```text
pub fn error_kind(messages: &[String]) -> FetchError   // src/check.rs, messages lowercased,
                                                         // outermost first, URL stripped
enum FetchError { Dns, DnsFailed, Timeout, Tls, Connect, NoLocation(..), Loop(..), TooManyHops(..) }
FetchError::DnsFailed displays as "dns lookup failed"
```

### Decisions
#### D1: Fail safe: dead only on a definite "no such name" answer (assumption)
Options: list the temporary messages and keep every other DNS error dead; list the definite
messages and make every other DNS error unverifiable. Chosen: the second. Why: a dead verdict
leads to a write, and resolvers word failures in many ways; a message nobody listed should not
rewrite a live link. Cost if wrong: a gone domain whose resolver words "no such name" in a way the
list misses shows as `UNVERIFIABLE dns lookup failed` and isn't fixed. The text report still names
it, and one phrase added to the list fixes it.

#### D2: The definite phrases (assumption)
`name or service not known` (glibc), `nodename nor servname provided` (macOS), `no such host`,
and `no address associated with hostname` (the name exists but has no address). A message is
definite if any of its sources contains one of them, even when it also says `dns error` or
`failed to lookup address`.

#### D3: A failed lookup is retried like a refused connection (assumption)
`DnsFailed` joins `Timeout` and `Connect` as a transient failure in `fetch_with_retries`, with the
same retry count. Why: a hiccup is often gone a second later, and a retried link that then answers
is checked normally.

#### D4: This settles the backlog's EAI_AGAIN line
The `[idea]` "Decide whether a temporary resolver failure (EAI_AGAIN …) is DEAD … or unverifiable"
is settled as unverifiable, and the line leaves the backlog.

### Scope
In:
- `error_kind` split out of `map_err`, the `DnsFailed` kind, its retry and its verdict, ~25 lines
  in `src/check.rs`: every weekly run on the runner.
- Docs: the README's dead and unverifiable reasons, `docs/architecture.md` (the error kinds, a
  Decisions line), ~10 lines.

Out: a DNS resolver of our own or `hickory-dns`; telling NXDOMAIN from SERVFAIL by DNS response
code; a config option for the phrase list; changing how timeouts, TLS or connection errors are
classified; the run-state idea (dead only after N runs).

Size: ~35 non-test lines in total.
Touches: none.

### Open for you
none

## Spec

### Requirements
- R1: WHEN a fetch fails and any of its error messages contains `name or service not known`,
  `nodename nor servname provided`, `no such host` or `no address associated with hostname`
  (ignoring case) THE SYSTEM SHALL give the kind `FetchError::Dns`, which stays dead with reason
  `dns`.
- R2: WHEN a fetch fails, no message contains an R1 phrase, and a message contains `dns error`,
  `failed to lookup address` or `temporary failure in name resolution` (ignoring case) THE SYSTEM
  SHALL give the kind `FetchError::DnsFailed`.
- R3: WHEN a fetch fails with `FetchError::DnsFailed` THE SYSTEM SHALL retry it as a transient
  failure, the same way and as many times as `FetchError::Connect`.
- R4: WHEN a URL's final outcome is `FetchError::DnsFailed` THE SYSTEM SHALL classify it
  unverifiable with reason `dns lookup failed`, look up no archived copy and propose no fix.
- R5: THE SYSTEM SHALL keep classifying the other failures as today: messages with `certificate`,
  `tls`, `ssl` or `handshake` give `Tls`, a timeout gives `Timeout`, and anything else gives
  `Connect`.

### Acceptance scenarios
S1–S4 drive the library, `rotguard::check`, since no test can make a real resolver fail on cue.
An unresolvable `.invalid` host staying `DEAD dns` end to end is already the locked test
`s11_unresolvable_host_is_dead_dns`.

- S1 [R1]: Given each of these message lists, when `error_kind` reads it, then the kind is
  `FetchError::Dns`:
  - `["dns error", "failed to lookup address information: name or service not known"]`
  - `["dns error", "failed to lookup address information: nodename nor servname provided, or
    not known"]` (one string)
  - `["no such host is known."]`
  - `["dns error", "failed to lookup address information: no address associated with hostname"]`
- S2 [R2]: Given each of these message lists, when `error_kind` reads it, then the kind is
  `FetchError::DnsFailed`:
  - `["dns error", "failed to lookup address information: temporary failure in name resolution"]`
  - `["dns error", "failed to lookup address information: try again"]`
  - `["dns error"]`
- S3 [R5]: Given `["error sending request", "invalid peer certificate: unknown issuer"]`, when
  `error_kind` reads it, then the kind is `FetchError::Tls`; and given
  `["error sending request", "connection refused"]`, the kind is `FetchError::Connect`.
- S4 [R4]: Given the outcome `Err(FetchError::DnsFailed)` and a default `HostConfig`, when
  `classify` runs, then the verdict is `Verdict::Unverifiable { reason: "dns lookup failed" }`.
- S5 [R3]: Given the outcome `Err(FetchError::DnsFailed)`, when `retry_for` decides what to do,
  then it answers `Retry::Transient`, the same as for `Err(FetchError::Connect)`. (A unit test in
  `src/check.rs`, since `retry_for` is private.)

### Details
- `error_kind` takes the lowercased messages that `map_err` builds today (the error without its
  URL, then each source). `map_err` keeps its timeout check first, then calls `error_kind`.
  Order inside `error_kind`: R1 phrases, then R2 phrases, then the TLS words, then `Connect`.
- `retry_for` (src/check.rs:705) answers `Retry::Transient` for `FetchError::DnsFailed` too, so
  `fetch_with_retries` retries it up to `TRANSIENT_RETRIES` times.
- `classify` gains no arm for `DnsFailed`: its existing `Err(err) => unverifiable(err.to_string())`
  gives the reason `dns lookup failed` from `Display`.
- `check_one` rechecks only a first `Dead`, so a `DnsFailed` outcome is not rechecked after
  `recheck_delay_ms`; the transient retries take its place.
- Reports: no code change. Text prints `UNVERIFIABLE dns lookup failed`, JSON `"class":
  "unverifiable", "reason": "dns lookup failed"`, markdown `could not check (dns lookup failed)`.

### Risk paths
none touched; `.whittle/gates.toml` unchanged.

### Fitness rules
Unchanged: only `src/apply.rs` writes files; tests use no real-network URLs;
`scripts/privacy-check.sh --all`.

### Review changes
- Round 1: builder check only; it confirmed the macOS no-such-name message with reqwest 0.13.5 and found nothing blocking.
