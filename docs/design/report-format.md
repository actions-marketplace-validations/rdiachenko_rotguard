---
status: approved
---
# Structured report formats

## Problem
rotguard prints its report only as aligned text. That is fine to read in a terminal, but a script
or a future CI job (the weekly-Action idea in the backlog) has to scrape it. The text also drops
data a reader may want, such as the URL of each redirect hop. And a report you want to keep or
open in a browser has no clickable links. The user wants a flag that prints the same report as
JSON, for scripts, or as HTML, for reading. A typical run on rdiachenko.com reports 0–10 findings
out of about 90 URLs, and nothing consumes the output today.

## Scope
In:
- `--format <text|json|html>`, ~10 lines; the default `text` is the report printed today.
- JSON report, ~45 lines with `serde_json` (chosen by the user): for scripts and CI.
- HTML report, ~90 lines with no new dependency (chosen by the user): a self-contained page for
  reading, written to stdout so the user redirects it (`> report.html`).
- README: document `--format`, and fix the sample header `PERMANENT REDIRECT 301` to
  `PERMANENT REDIRECT 301 200`, the text rotguard actually prints (docs, no code).

Out:
- Writing a report file from rotguard (`--output`): only `src/apply.rs` writes files (fitness
  rule), and `>` does the job.
- Other formats (Markdown, SARIF, JUnit), styling options, JavaScript, and a JSON schema version
  field. Each can be added when a consumer needs it.
- Any change to the text report, exit codes, progress output or `--apply`.

Size: ~145 non-test lines in total.

## Requirements
- R1: WHERE `--format` is not given, or is `text`, THE SYSTEM SHALL print exactly the report it
  prints today.
- R2: WHEN `--format json` is given THE SYSTEM SHALL print to stdout one JSON object
  `{"findings": [...], "summary": {...}}` followed by a newline, and nothing else. `findings` holds
  the same occurrences, in the same order, as the text report: every non-ok link, sorted by file
  and then line. Each finding has exactly these keys:
  - `file`: the path relative to the site directory, with `/` separators;
  - `line`;
  - `url`: the URL as written in the page;
  - `class`: one of `permanent`, `temporary`, `dead`, `suspect` or `unverifiable`;
  - `chain`: a list of `{"url", "status"}` hops, empty for dead and unverifiable links;
  - `reason`: a string for dead, suspect and unverifiable links, otherwise `null`;
  - `final_url`: the end of the chain for a temporary redirect, otherwise `null`;
  - `fix`.

  `summary` has the keys `checked`, `ok`, `permanent`, `temporary`, `dead`, `suspect`,
  `unverifiable` and `ignored`, with the same counts as the text summary line.
- R3: THE SYSTEM SHALL write each finding's `fix` as one of: `null` when there is no fix (a
  temporary redirect or an unverifiable link); `{"kind": "proposed", "url": …}` for a fix not
  written; `{"kind": "applied", "url": …}` for a fix `--apply` wrote; or
  `{"kind": "manual", "reason": …}`, where the reason is the text the report prints after
  `manual fix: `.
- R4: WHEN `--format html` is given THE SYSTEM SHALL print to stdout one self-contained HTML
  document. It has a `<title>` of `rotguard report`, the text summary line, and a table with one
  row per finding in the text report's order: the header cells are one `<thead><tr>` of `<th>`,
  and each finding is one `<tr>` inside `<tbody>`. The columns are Location (`file:line`), Class (the
  text header's class and its statuses or reason), URL, and Fix (the text detail line). Every
  http(s) URL in the URL and Fix columns is an `<a href>` link. With no finding, the table is
  replaced by the sentence `No broken links.` The page loads nothing (no `<script>`, `<link>`,
  `<img>` or remote URL other than the report's own links) and carries its CSS in one
  `<style>` element.
- R5: THE SYSTEM SHALL HTML-escape every piece of text in the HTML report that comes from the
  site or the network (file names, URLs, reasons), turning `&`, `<`, `>`, `"` and `'` into
  `&amp;`, `&lt;`, `&gt;`, `&quot;` and `&#39;`, in text and in `href` values alike.
- R6: THE SYSTEM SHALL keep `--format` from changing the exit code, the progress lines on stderr,
  the error messages on stderr or what `--apply` writes.
- R7: IF `--format` has a value other than `text`, `json` or `html` THEN THE SYSTEM SHALL exit
  with code 2, print a usage error on stderr naming the valid values, print nothing on stdout and
  make no request.

## Acceptance scenarios
- S1 [R1]: Given `post.md` with the line `[x](M/gone)`, and `M/gone` answering 404 twice, when
  rotguard runs once with no `--format` and once with `--format text`, then both runs print the
  same stdout, and that stdout has the entry `post.md:1  DEAD 404  M/gone`.
- S2 [R2, R3]: Given `post.md` whose line 1 is `[a](M/old)` and whose line 3 is `[b](M/gone)`,
  where `M/old` answers 301 → `M/new` → 200 and `M/gone` answers 404 twice with one CDX capture at
  20200101000000, when rotguard runs with `--format json`, then stdout parses as one JSON value,
  and:
  - `findings[0]` is `{"file": "post.md", "line": 1, "url": M/old, "class": "permanent",
    "chain": [{"url": M/old, "status": 301}, {"url": M/new, "status": 200}], "reason": null,
    "final_url": null, "fix": {"kind": "proposed", "url": M/new}}`;
  - `findings[1]` has `"class": "dead"`, `"chain": []`, `"reason": "404"` and
    `"fix": {"kind": "proposed", "url": <the snapshot URL the text report proposes>}`;
  - `summary` is `{"checked": 2, "ok": 0, "permanent": 1, "temporary": 0, "dead": 1,
    "suspect": 0, "unverifiable": 0, "ignored": 0}`.
- S3 [R3]: Given `post.md` with YAML front matter `source: "M/old"` and the body line
  `[x](M/old2)`, both answering 301 → `M/new` → 200, when rotguard runs with
  `--apply --format json`, then the finding for `M/old2` has
  `"fix": {"kind": "applied", "url": M/new}`, and the finding for `M/old` has
  `"fix": {"kind": "manual", "reason": "not verbatim"}`.
- S4 [R4]: Given the site of S2, when rotguard runs with `--format html`, then:
  - stdout starts with `<!doctype html>` (any case) and contains `<title>rotguard report</title>`;
  - stdout contains the text summary line and `<a href="M/new">`;
  - the `<tbody>` holds exactly two `<tr>` rows, in the S2 order;
  - stdout contains no `<script`, `<link` or `<img`.
- S5 [R4]: Given `post.md` whose only link `M/a` answers 200, when rotguard runs with
  `--format html`, then stdout contains `No broken links.` and no `<table`.
- S6 [R5]: Given `post.md` with the line `[x](M/x?a=1&b='2')`, and that URL answering 404 twice,
  when rotguard runs with `--format html`, then stdout contains `a=1&amp;b=&#39;2&#39;` and never
  contains `&b='2'`.
- S7 [R6]: Given `post.md` with `[x](M/gone)`, `M/gone` answering 404 twice and no capture, when
  rotguard runs with `--format json`, then it exits 1, as the text run does. Stderr has the
  `checking 1 link on 1 host` line, and all of stdout parses as one JSON value.
- S8 [R7]: Given any site, when rotguard runs with `--format yaml`, then it exits 2, stderr
  contains `text`, `json` and `html`, stdout is empty and `M` receives no request.

## Design

```text
            ┌──────────────────────── rotguard ────────────────────────┐
  site ────►│ lib::run ── findings, summary ──► report::render(fmt, …)  │──report──► stdout
  files     │   │ (after --apply, so fix kinds are final)   │           │
            │   └── progress, errors ──────────────────────┼───────────┼──────────► stderr
            │                       text: today's render   │           │
            │                       json: rows → serde_json│           │
            │                       html: rows → escaped   │           │
            └──────────────────────────────────────────────┴───────────┘
```

- `Args` gains `#[arg(long, value_enum, default_value_t = Format::Text)] format: Format`, with
  `#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)] pub enum Format { Text, Json, Html }`
  (it needs `Args`'s derives). An invalid value is
  rejected by clap before config load: exit 2 and a usage error naming the values (R7).
- `run` replaces `println!("{}", report::render(&findings, &summary))` with
  `println!("{}", report::render_as(format, &findings, &summary))`. Nothing else in `run` changes.
- In src/report.rs, `render` stays as it is for text. A private view shared by JSON and HTML:

  ```rust
  #[derive(Serialize)]
  struct Row<'a> {
      file: String,              // components joined with '/', as the text report does
      line: u32,
      url: &'a str,
      class: &'static str,       // "permanent" | "temporary" | "dead" | "suspect" | "unverifiable"
      chain: &'a [Hop],          // Hop already derives Serialize: {"url", "status"}
      reason: Option<&'a str>,
      final_url: Option<&'a Url>,
      fix: Option<FixView<'a>>,  // #[serde(tag = "kind", rename_all = "lowercase")]
  }
  enum FixView<'a> { Proposed { url: &'a str }, Applied { url: &'a str }, Manual { reason: String } }
  ```

  `rows(findings)` builds the rows in the text order (non-ok only, sorted by file then line).
  `Summary` gains `#[derive(Serialize)]`. `render_json` is
  `serde_json::to_string_pretty(&json!({"findings": rows, "summary": summary}))`, with its error
  `expect`ed: the types can't fail to serialize. Dead and unverifiable rows get an empty chain.
- `render_html` writes `<!doctype html>`, `<meta charset="utf-8">`, the title, one `<style>`
  block (~10 lines: a font, table borders), a `<p>` with the summary line, then either the table
  or `<p>No broken links.</p>`. One `escape(&str) -> String` function covers the five characters
  of R5 and is applied to every interpolated value. Class and Fix cells reuse the text wording
  (`DEAD 404`, `fix: <a href=…>…</a>`, `manual fix: not verbatim`, `final: <a …>`), so the two
  formats say the same thing.
- Dependencies: `serde_json` moves from transitive to a direct `[dependencies]` entry (it is
  already in Cargo.lock through the dev-dependencies). The acceptance tests parse JSON with it,
  as a normal dependency visible to tests.
- Failure: there is no new I/O and no new failure path. stdout errors behave as today.

## Decisions
### D1: Two machine and human formats behind one `--format` flag
Options: `--json` and `--html` booleans; one `--format` value. Chosen: `--format`. Why: one flag
can't be given twice with conflicting values, and a later format is one enum variant. Cost if
wrong: low.

### D2: JSON through `serde_json` (chosen by the user)
Options: hand-written JSON; `serde_json`. Chosen: `serde_json`. Why: correct escaping for any URL,
fewer lines, and it is already compiled for the tests. Cost if wrong: low; it touches `Cargo.toml`
and `Cargo.lock`, which the plan lists as risk paths.

### D3: Stable JSON keys, always present
Options: omit absent fields; always write every key, with `null`. Chosen: always present. Why: a
script reads `f["fix"]` without a key check, and the shape documents itself. Cost if wrong: low.

### D4: The HTML report goes to stdout, not a file
Options: `--output report.html`; stdout. Chosen: stdout. Why: the fitness rule keeps file writes
in `src/apply.rs`, and `> report.html` is enough. Cost if wrong: low; an `--output` can come later.

### D5: The backlog's R19 idea stays text-only
The `[idea]` that an unwritten fix prints `fix:` like an applied one is settled for JSON: R3's
`kind` tells `proposed` from `applied`. The text and HTML wording stays as it is (out of scope).

## Risk paths
`Cargo.toml` and `Cargo.lock` (for `serde_json`), both already in `.whittle/gates.toml`.

## Fitness rules
Unchanged: only `src/apply.rs` writes files; tests use no real-network URLs;
`scripts/privacy-check.sh --all`.

## Review changes
- Round 1: `Format` derives `Debug, PartialEq, Eq` too, so `Args` keeps compiling.
- Round 1: R4 fixes the table markup (`<thead><tr><th>` header, one `<tbody>` `<tr>` per finding), and S4 counts the `<tbody>` rows.

## Open for you
none
