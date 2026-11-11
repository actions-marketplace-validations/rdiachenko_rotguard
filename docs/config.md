# Configuration reference

rotguard reads `rotguard.toml` from the site directory (the directory you run it on). Only `site_url` is required; every other key has a default. An unknown key, a value of the wrong type or a missing `site_url` is an error, so a typo never passes silently. Every error prints `rotguard: ` and a message to stderr and exits with code 2. The complete file at its defaults is at the end of this page.

### `site_url`

- Type: string (a URL). Required, no default.
- Valid values: an `http` or `https` URL with a host.
- Effect: links whose host equals the host of `site_url` are internal and are not checked. Only the host is compared, so scheme and port are ignored. There is no `www.` folding and no subdomain match: `www.example.com` and `example.com` are different hosts.
- Error: a missing key, a non-string value, invalid TOML, or a value that is not an absolute URL or has an empty host (`"example.com"`, `"https://"`) prints the generic `invalid rotguard.toml: ...` message. A parseable URL with another scheme (for example `ftp://example.com`) prints `` invalid rotguard.toml: `site_url` (<value>) must be an http or https URL with a host ``.

### `content`

- Type: array of strings (globs).
- Default: `["content/**/*.md"]`.
- Valid values: glob patterns. A file is read when its path matches any pattern.
- Effect: patterns are matched against paths relative to the site directory. `*` stays within one path segment; `**` crosses segments. Only files are read, in sorted path order.
- Error: a pattern that is not a valid glob prints `rotguard: content glob "<pattern>": ...`. A wrong type prints `invalid rotguard.toml`.

### `date_key`

- Type: string.
- Default: `"date"`.
- Valid values: any front-matter key name.
- Effect: the key read from a page's TOML or YAML front matter to date its links. The value must be a TOML date or datetime, or a string starting with `YYYY-MM-DD`. The archive snapshot used is the latest capture on or before that date, or, if there is none, the earliest capture after it; a page without a readable date uses the day of the run.
- Error: a wrong type prints `invalid rotguard.toml`.

### `user_agent`

- Type: string.
- Default: `"rotguard/<version>"`, which follows the installed version.
- Valid values: any string that can be sent as an HTTP header value (no control characters or newlines).
- Effect: sent as the `User-Agent` of link checks, soft-404 probes and archive queries.
- Error: an invalid value prints `` invalid rotguard.toml: `user_agent` ("<value>") is not a valid HTTP header value ``.

### `ignore`

- Type: array of strings (regular expressions).
- Default: `[]`.
- Valid values: regexes in the Rust `regex` syntax.
- Effect: an unanchored regex search on the URL as written in the page, fragment included. A URL matching any entry is not checked. This runs after the internal-host check, so internal links never count as ignored. URLs that do not parse are skipped as well. Ignored URLs are counted once each (without their fragment) as `ignored` in the summary.
- Error: a bad regex prints ``invalid rotguard.toml: bad regex in `ignore` ("<pattern>")``. A wrong type prints `invalid rotguard.toml`.

### `not_found_phrases`

- Type: array of strings.
- Default: `["not found", "404", "page does not exist", "page doesn't exist"]`.
- Valid values: any strings. Empty phrases are ignored. Set `[]` to turn the title check off.
- Effect: a case-insensitive substring match on the page `<title>`, with whitespace in a phrase collapsed to single spaces. It applies only to 2xx pages on hosts with `soft404` on, and is skipped when the probe for the page's parent path answered 404 or 410. A match makes the link SUSPECT.
- Error: a wrong type prints `invalid rotguard.toml`.

### `archive_url`

- Type: string (a URL).
- Default: `"https://web.archive.org"`.
- Valid values: an `http` or `https` URL with a host.
- Effect: the base of the archive. Snapshot lookups go to `<archive_url>/cdx/search/cdx`, and proposed fixes are `<archive_url>/web/<timestamp>/<url>`. A trailing `/` is ignored.
- Error: a value that is not an absolute URL or has an empty host (`"example.com"`, `"https://"`), or a wrong type, prints the generic `invalid rotguard.toml: ...` message. A parseable URL with another scheme (for example `ftp://example.com`) prints `` invalid rotguard.toml: `archive_url` (<value>) must be an http or https URL with a host ``.

### `host_interval_ms`

- Type: integer, milliseconds.
- Default: `1000`.
- Valid values: `0` or more.
- Effect: the minimum time between the starts of two requests to the same host. It counts every request: redirect hops, retries, rechecks, soft-404 probes and the request for a fix stripped of `utm_*` pairs. `0` turns the spacing off.
- Error: a negative number or a wrong type prints `invalid rotguard.toml`.

### `archive_interval_ms`

- Type: integer, milliseconds.
- Default: `4000`.
- Valid values: `0` or more.
- Effect: one global minimum time between the starts of archive requests, whatever the URL.
- Error: a negative number or a wrong type prints `invalid rotguard.toml`.

### `recheck_delay_ms`

- Type: integer, milliseconds.
- Default: `5000`.
- Valid values: `0` or more.
- Effect: the wait before a URL that came back dead (404, 410 or a host DNS says doesn't exist) is fetched once more. The second answer decides the verdict.
- Error: a negative number or a wrong type prints `invalid rotguard.toml`.

### `timeout_ms`

- Type: integer, milliseconds.
- Default: `10000`.
- Valid values: `0` or more.
- Effect: the time limit of each request, not of a whole redirect chain: every hop gets its own. The same value applies to archive requests. `0` is a limit of zero, not "no limit": every request times out, so every URL is UNVERIFIABLE (timeout).
- Error: a negative number or a wrong type prints `invalid rotguard.toml`.

### `deadline_s`

- Type: integer, seconds.
- Default: `300`.
- Valid values: `0` to `86400` (one day).
- Effect: the limit for checking and archive lookups together, counted from the start of checking. A URL not finished by then is reported as unverifiable (`deadline`), and a lookup not finished has no fix. `0` is a limit of zero, not "no limit": every URL is UNVERIFIABLE (deadline).
- Error: a larger value prints `` invalid rotguard.toml: `deadline_s` (<value>) must be at most 86400 ``. A negative number or a wrong type prints `invalid rotguard.toml`.

### `[hosts."<name>"]`

- Type: table of tables, keyed by host name. Default: empty.
- Valid keys: `soft404` (boolean, default `true`). Any other key prints `invalid rotguard.toml`.
- Effect: `soft404 = false` turns soft-404 detection off for URLs on that host: no soft-404 probe request, no `not_found_phrases` check, and a redirect to the site root is not suspect. With it on, detection may also GET the host's root once and can report UNVERIFIABLE (`probe: every URL gets the same page`, `probe: page needs JavaScript`); `false` turns those off. A 401, 403 or 999 on that host still gets the bot-wall probe. A 404 or 410 on that host that stays dead after the recheck still gets the root check: it may GET the host's root once and can report UNVERIFIABLE (`404, host root answers 404 too`) when the root answers 404 or 410 too. A host with no entry has `soft404` on.
- Matching: keys are lowercased and matched exactly against the host of the URL as written in the page. `www.` is not folded and subdomains are not matched.
- Error: two keys that are equal once lowercased print `` invalid rotguard.toml: `[hosts]` lists "<host>" more than once (keys ignore case) ``.

## All keys at their defaults

```toml
site_url = "https://example.com"
content = ["content/**/*.md"]
date_key = "date"
user_agent = "rotguard/<version>"
ignore = []
not_found_phrases = ["not found", "404", "page does not exist", "page doesn't exist"]
archive_url = "https://web.archive.org"
host_interval_ms = 1000
archive_interval_ms = 4000
recheck_delay_ms = 5000
timeout_ms = 10000
deadline_s = 300
```
