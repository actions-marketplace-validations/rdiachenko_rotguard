//! Formats the findings and the summary line printed on stdout.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt::{self, Write as _};

use serde::Serialize;
use url::Url;

use crate::apply::Unwritable;
use crate::archive::NoFix;
use crate::check::{Hop, Verdict};
use crate::{Finding, Fix, Format, ManualReason};

/// Counts of distinct URLs by outcome, computed by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Summary {
    pub checked: usize,
    pub ok: usize,
    pub permanent: usize,
    pub temporary: usize,
    pub dead: usize,
    pub suspect: usize,
    pub unverifiable: usize,
    pub blocked: usize,
    pub ignored: usize,
}

/// Renders the report in `format`.
#[must_use]
pub fn render_as(format: Format, findings: &[Finding], summary: &Summary) -> String {
    match format {
        Format::Text => render(findings, summary),
        Format::Json => render_json(findings, summary),
        Format::Html => render_html(findings, summary),
        Format::Markdown => render_markdown(findings, summary),
    }
}

/// Renders one entry per non-ok finding, sorted by file then line, then the summary line.
///
/// Findings whose verdict is `Ok` are not reported. The result has no trailing newline.
#[must_use]
pub fn render(findings: &[Finding], summary: &Summary) -> String {
    let mut out = String::new();
    for finding in reported(findings) {
        write_entry(&mut out, finding);
    }
    out.push_str(&summary_line(summary));
    out
}

fn summary_line(summary: &Summary) -> String {
    let Summary {
        checked,
        ok,
        permanent,
        temporary,
        dead,
        suspect,
        unverifiable,
        blocked,
        ignored,
    } = summary;
    format!(
        "checked {checked}: ok {ok}, permanent {permanent}, temporary {temporary}, \
         dead {dead}, suspect {suspect}, unverifiable {unverifiable}, \
         blocked {blocked}, ignored {ignored}"
    )
}

/// The non-ok findings, sorted by file then line.
fn reported(findings: &[Finding]) -> Vec<&Finding> {
    let mut sorted: Vec<&Finding> = findings
        .iter()
        .filter(|finding| !matches!(finding.verdict, Verdict::Ok { .. }))
        .collect();
    sorted.sort_by(|a, b| (&a.file, a.link.line).cmp(&(&b.file, b.link.line)));
    sorted
}

/// The path with `/` separators on every platform.
fn display_path(finding: &Finding) -> String {
    finding
        .file
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Serialize)]
struct Report<'a> {
    findings: Vec<Row<'a>>,
    summary: &'a Summary,
}

/// One finding as it appears in the JSON report.
#[derive(Serialize)]
struct Row<'a> {
    file: String,
    line: u32,
    url: &'a str,
    class: &'static str,
    chain: &'a [Hop],
    reason: Option<Cow<'a, str>>,
    final_url: Option<&'a Url>,
    fix: Option<FixView<'a>>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum FixView<'a> {
    Proposed { url: &'a str },
    Applied { url: &'a str },
    Manual { reason: String },
}

impl<'a> From<&'a Fix> for Option<FixView<'a>> {
    fn from(fix: &'a Fix) -> Self {
        match fix {
            Fix::None => None,
            Fix::Proposed(url) => Some(FixView::Proposed { url }),
            Fix::Applied(url) => Some(FixView::Applied { url }),
            Fix::Manual(reason) => Some(FixView::Manual {
                reason: reason.to_string(),
            }),
        }
    }
}

fn rows(findings: &[Finding]) -> Vec<Row<'_>> {
    reported(findings)
        .into_iter()
        .map(|finding| {
            let (class, chain, reason, final_url) = match &finding.verdict {
                // `reported` drops ok findings.
                Verdict::Ok { .. } => unreachable!("ok findings are not reported"),
                Verdict::Permanent { chain, .. } => ("permanent", chain.as_slice(), None, None),
                Verdict::Blocked { status } => (
                    "blocked",
                    [].as_slice(),
                    Some(Cow::Owned(status.to_string())),
                    None,
                ),
                Verdict::Temporary { chain, final_url } => {
                    ("temporary", chain.as_slice(), None, Some(final_url))
                }
                Verdict::Dead { reason } => (
                    "dead",
                    [].as_slice(),
                    Some(Cow::Borrowed(reason.as_str())),
                    None,
                ),
                Verdict::Suspect { chain, reason } => (
                    "suspect",
                    chain.as_slice(),
                    Some(Cow::Borrowed(reason.as_str())),
                    None,
                ),
                Verdict::Unverifiable { reason } => (
                    "unverifiable",
                    [].as_slice(),
                    Some(Cow::Borrowed(reason.as_str())),
                    None,
                ),
            };
            Row {
                file: display_path(finding),
                line: finding.link.line,
                url: &finding.link.url,
                class,
                chain,
                reason,
                final_url,
                fix: (&finding.fix).into(),
            }
        })
        .collect()
}

/// Renders `{"findings": [...], "summary": {...}}` as pretty JSON, without a trailing newline.
#[must_use]
pub fn render_json(findings: &[Finding], summary: &Summary) -> String {
    let report = Report {
        findings: rows(findings),
        summary,
    };
    serde_json::to_string_pretty(&report).expect("the report types always serialize")
}

/// The detail line under a finding: a labelled link, or a manual-fix reason.
enum Detail {
    Link { label: &'static str, url: String },
    Manual(String),
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Link { label, url } => write!(f, "{label}: {url}"),
            Self::Manual(reason) => write!(f, "manual fix: {reason}"),
        }
    }
}

/// The class wording and the detail line of a reported finding.
fn describe(finding: &Finding) -> Option<(String, Option<Detail>)> {
    Some(match &finding.verdict {
        Verdict::Ok { .. } => return None,
        Verdict::Permanent { chain, .. } => (
            format!("PERMANENT REDIRECT {}", statuses(chain)),
            fix_detail(&finding.fix),
        ),
        Verdict::Temporary { chain, final_url } => (
            format!("TEMPORARY REDIRECT {}", statuses(chain)),
            Some(Detail::Link {
                label: "final",
                url: final_url.to_string(),
            }),
        ),
        Verdict::Dead { reason } => (format!("DEAD {reason}"), fix_detail(&finding.fix)),
        Verdict::Suspect { reason, .. } => (format!("SUSPECT {reason}"), fix_detail(&finding.fix)),
        Verdict::Unverifiable { reason } => (format!("UNVERIFIABLE {reason}"), None),
        Verdict::Blocked { status } => (format!("BLOCKED {status}"), None),
    })
}

fn write_entry(out: &mut String, finding: &Finding) {
    let Some((class, detail)) = describe(finding) else {
        return;
    };
    let file = display_path(finding);
    writeln!(
        out,
        "{file}:{}  {class}  {}",
        finding.link.line, finding.link.url
    )
    .expect("writing to a String cannot fail");
    if let Some(detail) = detail {
        writeln!(out, "    {detail}").expect("writing to a String cannot fail");
    }
}

fn statuses(chain: &[Hop]) -> String {
    chain
        .iter()
        .map(|hop| hop.status.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn fix_detail(fix: &Fix) -> Option<Detail> {
    match fix {
        Fix::None => None,
        Fix::Proposed(url) | Fix::Applied(url) => Some(Detail::Link {
            label: "fix",
            url: url.clone(),
        }),
        Fix::Manual(reason) => Some(Detail::Manual(reason.to_string())),
    }
}

/// `s` with `&`, `<`, `>`, `"` and `\'` replaced by their HTML entities.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn link(url: &str) -> String {
    let url = escape(url);
    format!("<a href=\"{url}\">{url}</a>")
}

const STYLE: &str = "body{font-family:sans-serif;margin:1rem}\
table{border-collapse:collapse}\
th,td{border:1px solid #888;padding:.25rem .5rem;text-align:left;vertical-align:top}";

/// Renders one self-contained HTML page: the summary line, then a table of the reported
/// findings in the text report's order, or `No broken links.`. No trailing newline.
#[must_use]
pub fn render_html(findings: &[Finding], summary: &Summary) -> String {
    let mut out = format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<title>rotguard report</title>\n\
         <style>{STYLE}</style>\n<p>{}</p>\n",
        escape(&summary_line(summary))
    );
    let entries: Vec<_> = reported(findings)
        .into_iter()
        .filter_map(|finding| describe(finding).map(|(class, detail)| (finding, class, detail)))
        .collect();
    if entries.is_empty() {
        out.push_str("<p>No broken links.</p>");
        return out;
    }
    out.push_str(
        "<table><thead><tr><th>Location</th><th>Class</th><th>URL</th><th>Fix</th></tr></thead>\n<tbody>\n",
    );
    for (finding, class, detail) in entries {
        let fix = match detail {
            None => String::new(),
            Some(Detail::Link { label, url }) => format!("{label}: {}", link(&url)),
            Some(manual @ Detail::Manual(_)) => escape(&manual.to_string()),
        };
        writeln!(
            out,
            "<tr><td>{}:{}</td><td>{}</td><td>{}</td><td>{fix}</td></tr>",
            escape(&display_path(finding)),
            finding.link.line,
            escape(&class),
            link(&finding.link.url),
        )
        .expect("writing to a String cannot fail");
    }
    out.push_str("</tbody></table>");
    out
}

/// `s` with markdown-special characters backslash-escaped and control characters as spaces.
fn md_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_control() {
            out.push(' ');
        } else {
            if "\\`*_[]<>()!|~&#".contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
    }
    out
}

/// `url` as an autolink when it is a plain http(s) URL, otherwise as escaped text.
fn md_link(url: &str) -> String {
    let plain = (url.starts_with("http://") || url.starts_with("https://"))
        && !url.contains(|c: char| c.is_control() || " <>|`\\".contains(c));
    if plain {
        format!("<{url}>")
    } else {
        md_escape(url)
    }
}

/// The Problem cell of a reported finding.
fn md_problem(verdict: &Verdict) -> String {
    match verdict {
        // `reported` drops ok findings.
        Verdict::Ok { .. } => unreachable!("ok findings are not reported"),
        Verdict::Permanent { chain, .. } => {
            format!("moved permanently ({})", md_escape(&statuses(chain)))
        }
        Verdict::Temporary { chain, final_url } => format!(
            "moved temporarily ({}) to {}",
            md_escape(&statuses(chain)),
            md_link(final_url.as_str())
        ),
        Verdict::Dead { reason } => format!("dead ({})", md_escape(reason)),
        Verdict::Suspect { reason, .. } => format!("suspect ({})", md_escape(reason)),
        Verdict::Unverifiable { reason } => format!("could not check ({})", md_escape(reason)),
        Verdict::Blocked { status } => {
            format!("blocked by a bot wall ({})", md_escape(&status.to_string()))
        }
    }
}

/// The reason cell of a fix that needs a hand edit.
fn md_reason(reason: ManualReason) -> &'static str {
    match reason {
        ManualReason::Archive(NoFix::NoCapture) => "no archived copy in the Wayback Machine",
        ManualReason::Archive(NoFix::Unreachable) => "the Wayback Machine didn't answer",
        ManualReason::Archive(NoFix::Deadline) => {
            "the run ran out of time looking for an archived copy"
        }
        ManualReason::Unwritable(Unwritable::NotVerbatim) => {
            "the file writes the URL differently from the link"
        }
        ManualReason::Unwritable(Unwritable::CannotQuote) => {
            "the new URL can't be written inside this link"
        }
        ManualReason::ChangedDuringRun => "the file changed while rotguard ran",
    }
}

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

/// The rows of one markdown section: findings merged by page and cells.
#[derive(Default)]
struct Section<'a> {
    /// Page, line numbers and the cells after the Lines cell.
    rows: Vec<(String, Vec<u32>, Vec<String>)>,
    places: usize,
    urls: HashSet<&'a str>,
}

impl<'a> Section<'a> {
    fn add(&mut self, finding: &'a Finding, cells: Vec<String>) {
        let page = display_path(finding);
        let line = finding.link.line;
        self.places += 1;
        self.urls.insert(&finding.link.url);
        match self
            .rows
            .iter_mut()
            .find(|row| row.0 == page && row.2 == cells)
        {
            Some(row) => row.1.push(line),
            None => self.rows.push((page, vec![line], cells)),
        }
    }

    fn heading(&self, title: &str) -> String {
        let (links, places) = (self.urls.len(), self.places);
        format!(
            "{title}: {links} {} in {places} {}",
            plural(links, "link", "links"),
            plural(places, "place", "places")
        )
    }

    fn table(&self, header: &str) -> String {
        let columns = header.matches('|').count() - 1;
        let mut out = format!("{header}\n|{}\n", "---|".repeat(columns));
        for (page, lines, cells) in &self.rows {
            let lines = lines.iter().map(u32::to_string).collect::<Vec<_>>();
            write!(out, "| {} | {} |", md_escape(page), lines.join(", "))
                .expect("writing to a String cannot fail");
            for cell in cells {
                write!(out, " {cell} |").expect("writing to a String cannot fail");
            }
            out.push('\n');
        }
        out.pop();
        out
    }
}

const FIX_HEADER: &str = "| Page | Lines | Old link | New link | Why |";

/// Renders the report as markdown for a pull request body: the findings grouped by what was
/// done about them, then the counts line. No trailing newline.
#[must_use]
pub fn render_markdown(findings: &[Finding], summary: &Summary) -> String {
    let mut fixed = Section::default();
    let mut to_apply = Section::default();
    let mut by_hand = Section::default();
    let mut folded = Section::default();
    let reported = reported(findings);
    let nothing = reported.is_empty();
    for finding in reported {
        let link = md_link(&finding.link.url);
        let problem = md_problem(&finding.verdict);
        match &finding.fix {
            Fix::Applied(url) => fixed.add(finding, vec![link, md_link(url), problem]),
            // `--apply` never writes a suspect's fix, so it always needs a hand.
            Fix::Proposed(url) if matches!(finding.verdict, Verdict::Suspect { .. }) => {
                let reason = format!(
                    "rotguard doesn't rewrite suspect links; archived copy: {}",
                    md_link(url)
                );
                by_hand.add(finding, vec![link, problem, reason]);
            }
            Fix::Proposed(url) => to_apply.add(finding, vec![link, md_link(url), problem]),
            Fix::Manual(reason) => {
                by_hand.add(finding, vec![link, problem, md_reason(*reason).into()]);
            }
            Fix::None => folded.add(finding, vec![link, problem]),
        }
    }
    let mut blocks = Vec::new();
    if nothing {
        blocks.push("No broken links.".to_owned());
    } else {
        if fixed.places > 0 {
            blocks.push(fixed.heading("## Fixed"));
            blocks.push(fixed.table(FIX_HEADER));
        }
        if to_apply.places > 0 {
            blocks.push(to_apply.heading("## Fixes to apply"));
            blocks.push("Run `rotguard --apply` to write them.".to_owned());
            blocks.push(to_apply.table(FIX_HEADER));
        }
        if by_hand.places > 0 {
            blocks.push(by_hand.heading("## Needs a hand fix"));
            blocks.push(
                by_hand.table("| Page | Lines | Link | Problem | Why rotguard couldn't fix it |"),
            );
        } else {
            blocks.push("## Needs a hand fix".to_owned());
            blocks.push("Nothing.".to_owned());
        }
        if folded.places > 0 {
            let links = folded.urls.len();
            blocks.push(format!(
                "<details><summary>{links} more {} to know about, no action needed</summary>",
                plural(links, "link", "links")
            ));
            blocks.push(folded.table("| Page | Lines | Link | What rotguard saw |"));
            blocks.push("</details>".to_owned());
        }
    }
    blocks.push(md_counts(summary));
    blocks.join("\n\n")
}

fn md_counts(summary: &Summary) -> String {
    let Summary {
        checked,
        ok,
        permanent,
        temporary,
        dead,
        suspect,
        unverifiable,
        blocked,
        ignored,
    } = *summary;
    let mut out = format!(
        "{checked} {} checked: {ok} ok",
        plural(checked, "link", "links")
    );
    let counts = [
        (permanent, "moved permanently"),
        (temporary, "moved temporarily"),
        (dead, "dead"),
        (suspect, "suspect"),
        (unverifiable, "could not check"),
        (blocked, "blocked"),
        (ignored, "ignored"),
    ];
    for (count, label) in counts {
        if count > 0 {
            write!(out, ", {count} {label}").expect("writing to a String cannot fail");
        }
    }
    out.push('.');
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::extract::{Ctx, Link};

    fn finding(file: &str, line: u32, verdict: Verdict, fix: Fix) -> Finding {
        Finding {
            file: file.into(),
            link: Link {
                url: "https://old.test/x#frag".into(),
                line,
                span: 0..0,
                ctx: Ctx::MdDest,
                verbatim: true,
            },
            verdict,
            fix,
        }
    }

    fn hop(url: &str, status: u16) -> Hop {
        Hop {
            url: Url::parse(url).unwrap(),
            status,
        }
    }

    fn permanent() -> Verdict {
        Verdict::Permanent {
            chain: vec![
                hop("https://old.test/x", 301),
                hop("https://new.test/x", 200),
            ],
            fix: Url::parse("https://new.test/x").unwrap(),
        }
    }

    fn dead() -> Verdict {
        Verdict::Dead {
            reason: "404".into(),
        }
    }

    fn summary() -> Summary {
        Summary {
            checked: 4,
            ok: 1,
            permanent: 1,
            dead: 1,
            suspect: 1,
            ..Summary::default()
        }
    }

    #[test]
    fn entries_sorted_by_file_then_line() {
        let findings = [
            finding("b.md", 7, dead(), Fix::None),
            finding("a.md", 9, dead(), Fix::None),
            finding("a.md", 2, dead(), Fix::None),
        ];
        let out = render(&findings, &summary());
        let locs: Vec<&str> = out
            .lines()
            .filter(|l| !l.starts_with(' ') && !l.starts_with("checked "))
            .map(|l| l.split("  ").next().unwrap())
            .collect();
        assert_eq!(locs, ["a.md:2", "a.md:9", "b.md:7"]);
    }

    #[test]
    fn each_class_prints_its_detail_line() {
        let chain = |a, b| vec![hop("https://old.test/x", a), hop("https://t.test/y", b)];
        let manual = || Fix::Manual(ManualReason::Archive(NoFix::NoCapture));
        let suspect = || Verdict::Suspect {
            chain: chain(301, 200),
            reason: "redirect to root".into(),
        };
        let cases = [
            (
                "permanent",
                finding(
                    "a.md",
                    3,
                    permanent(),
                    Fix::Proposed("https://new.test/x#frag".into()),
                ),
                &[
                    "a.md:3  PERMANENT REDIRECT 301 200  https://old.test/x#frag",
                    "    fix: https://new.test/x#frag",
                ][..],
            ),
            (
                "applied",
                finding(
                    "a.md",
                    3,
                    permanent(),
                    Fix::Applied("https://new.test/x".into()),
                ),
                &[
                    "a.md:3  PERMANENT REDIRECT 301 200  https://old.test/x#frag",
                    "    fix: https://new.test/x",
                ],
            ),
            (
                "dead",
                finding("a.md", 1, dead(), manual()),
                &[
                    "a.md:1  DEAD 404  https://old.test/x#frag",
                    "    manual fix: no archived copy",
                ],
            ),
            (
                "temporary",
                finding(
                    "a.md",
                    1,
                    Verdict::Temporary {
                        chain: chain(302, 200),
                        final_url: Url::parse("https://t.test/y").unwrap(),
                    },
                    Fix::None,
                ),
                &[
                    "a.md:1  TEMPORARY REDIRECT 302 200  https://old.test/x#frag",
                    "    final: https://t.test/y",
                ],
            ),
            (
                "unverifiable",
                finding(
                    "a.md",
                    1,
                    Verdict::Unverifiable {
                        reason: "deadline".into(),
                    },
                    Fix::None,
                ),
                &["a.md:1  UNVERIFIABLE deadline  https://old.test/x#frag"],
            ),
            (
                "blocked",
                finding("a.md", 1, Verdict::Blocked { status: 403 }, Fix::None),
                &["a.md:1  BLOCKED 403  https://old.test/x#frag"],
            ),
            (
                "suspect with fix",
                finding(
                    "a.md",
                    1,
                    suspect(),
                    Fix::Proposed("https://web.test/s".into()),
                ),
                &[
                    "a.md:1  SUSPECT redirect to root  https://old.test/x#frag",
                    "    fix: https://web.test/s",
                ],
            ),
            (
                "suspect manual",
                finding("a.md", 1, suspect(), manual()),
                &[
                    "a.md:1  SUSPECT redirect to root  https://old.test/x#frag",
                    "    manual fix: no archived copy",
                ],
            ),
        ];
        for (name, f, expected) in cases {
            let out = render(&[f], &Summary::default());
            let lines: Vec<&str> = out.lines().collect();
            // The expected entry lines, then straight to the summary: no extra detail line.
            assert_eq!(lines[..expected.len()], *expected, "{name}");
            assert!(
                lines[expected.len()].starts_with("checked "),
                "{name}: {out}"
            );
        }
    }

    #[test]
    fn json_rows_cover_each_class_and_fix_kind() {
        let chain = |a| vec![hop("https://old.test/x", a), hop("https://t.test/y", 200)];
        let hops = |a| json!([{"url": "https://old.test/x", "status": a}, {"url": "https://t.test/y", "status": 200}]);
        let findings = [
            finding(
                "d/u.md",
                1,
                Verdict::Unverifiable {
                    reason: "deadline".into(),
                },
                Fix::None,
            ),
            finding(
                "d/t.md",
                1,
                Verdict::Temporary {
                    chain: chain(302),
                    final_url: Url::parse("https://t.test/y").unwrap(),
                },
                Fix::None,
            ),
            finding(
                "d/s.md",
                1,
                Verdict::Suspect {
                    chain: chain(301),
                    reason: "redirect to root".into(),
                },
                Fix::Manual(ManualReason::Archive(NoFix::NoCapture)),
            ),
            finding(
                "d/p.md",
                1,
                Verdict::Permanent {
                    chain: chain(301),
                    fix: Url::parse("https://t.test/y").unwrap(),
                },
                Fix::Applied("https://t.test/y".into()),
            ),
            finding("d/o.md", 1, Verdict::Ok { chain: chain(200) }, Fix::None),
            finding("d/b.md", 1, Verdict::Blocked { status: 403 }, Fix::None),
            finding(
                "d/a.md",
                1,
                dead(),
                Fix::Proposed("https://web.test/s".into()),
            ),
        ];
        let out = render_json(&findings, &summary());
        assert!(!out.ends_with('\n'));
        let row = |file: &str,
                   class: &str,
                   chain: serde_json::Value,
                   reason: serde_json::Value,
                   final_url: serde_json::Value,
                   fix: serde_json::Value| {
            json!({"file": file, "line": 1, "url": "https://old.test/x#frag", "class": class,
                "chain": chain, "reason": reason, "final_url": final_url, "fix": fix})
        };
        let expected = json!({
            "findings": [
                row("d/a.md", "dead", json!([]), json!("404"), json!(null), json!({"kind": "proposed", "url": "https://web.test/s"})),
                row("d/b.md", "blocked", json!([]), json!("403"), json!(null), json!(null)),
                row("d/p.md", "permanent", hops(301), json!(null), json!(null), json!({"kind": "applied", "url": "https://t.test/y"})),
                row("d/s.md", "suspect", hops(301), json!("redirect to root"), json!(null), json!({"kind": "manual", "reason": "no archived copy"})),
                row("d/t.md", "temporary", hops(302), json!(null), json!("https://t.test/y"), json!(null)),
                row("d/u.md", "unverifiable", json!([]), json!("deadline"), json!(null), json!(null)),
            ],
            "summary": {"checked": 4, "ok": 1, "permanent": 1, "temporary": 0, "dead": 1,
                "suspect": 1, "unverifiable": 0, "blocked": 0, "ignored": 0},
        });
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&out).unwrap(),
            expected
        );
    }

    #[test]
    fn summary_line_is_last_without_trailing_newline() {
        let out = render(&[finding("a.md", 1, dead(), Fix::None)], &summary());
        assert_eq!(
            out.lines().last().unwrap(),
            "checked 4: ok 1, permanent 1, temporary 0, dead 1, suspect 1, unverifiable 0, blocked 0, ignored 0"
        );
        assert!(!out.ends_with('\n'));
    }

    #[test]
    fn escape_maps_all_five_characters() {
        assert_eq!(escape(r#"&<>"'"#), "&amp;&lt;&gt;&quot;&#39;");
    }

    #[test]
    fn html_page_has_a_row_per_finding_with_escaped_cells() {
        let findings = [
            finding(
                "a.md",
                1,
                permanent(),
                Fix::Proposed("https://new.test/x?a=1&b=2".into()),
            ),
            finding(
                "a.md",
                2,
                Verdict::Temporary {
                    chain: vec![hop("https://old.test/x", 302), hop("https://t.test/y", 200)],
                    final_url: Url::parse("https://t.test/y").unwrap(),
                },
                Fix::None,
            ),
            finding(
                "a.md",
                3,
                dead(),
                Fix::Manual(ManualReason::Archive(NoFix::NoCapture)),
            ),
            finding(
                "a.md",
                4,
                Verdict::Unverifiable {
                    reason: "<dead>".into(),
                },
                Fix::None,
            ),
            finding("a.md", 5, Verdict::Ok { chain: vec![] }, Fix::None),
            finding(
                "a.md",
                6,
                Verdict::Suspect {
                    chain: vec![
                        hop("https://old.test/x", 301),
                        hop("https://old.test/", 200),
                    ],
                    reason: "redirect to root".into(),
                },
                Fix::Proposed("https://web.test/snap".into()),
            ),
        ];
        let out = render_html(&findings, &summary());
        assert!(out.starts_with(
            "<!doctype html>\n<meta charset=\"utf-8\">\n<title>rotguard report</title>\n<style>"
        ));
        assert!(!out.ends_with('\n'));
        let body = out.split("<tbody>").nth(1).unwrap();
        assert_eq!(body.matches("<tr>").count(), 5);
        let a = r#"<a href="https://old.test/x#frag">https://old.test/x#frag</a>"#;
        let cases = [
            format!(
                "<tr><td>a.md:1</td><td>PERMANENT REDIRECT 301 200</td><td>{a}</td><td>fix: <a href=\"https://new.test/x?a=1&amp;b=2\">https://new.test/x?a=1&amp;b=2</a></td></tr>"
            ),
            format!(
                "<tr><td>a.md:2</td><td>TEMPORARY REDIRECT 302 200</td><td>{a}</td><td>final: <a href=\"https://t.test/y\">https://t.test/y</a></td></tr>"
            ),
            format!(
                "<tr><td>a.md:3</td><td>DEAD 404</td><td>{a}</td><td>manual fix: no archived copy</td></tr>"
            ),
            format!(
                "<tr><td>a.md:4</td><td>UNVERIFIABLE &lt;dead&gt;</td><td>{a}</td><td></td></tr>"
            ),
            format!(
                "<tr><td>a.md:6</td><td>SUSPECT redirect to root</td><td>{a}</td><td>fix: <a href=\"https://web.test/snap\">https://web.test/snap</a></td></tr>"
            ),
        ];
        for case in cases {
            assert!(out.contains(&case), "{case}\n{out}");
        }
        assert!(out.contains(&format!("<p>{}</p>", summary_line(&summary()))));
    }

    #[test]
    fn html_page_without_findings_says_so() {
        let out = render_html(
            &[finding("a.md", 1, Verdict::Ok { chain: vec![] }, Fix::None)],
            &summary(),
        );
        assert!(out.ends_with("<p>No broken links.</p>"));
        assert!(!out.contains("<table"));
    }

    #[test]
    fn markdown_sections_merge_rows_and_fold_the_rest() {
        let applied = || Fix::Applied("https://new.test/x#frag".into());
        let manual = |r| Fix::Manual(r);
        let mut other = finding("a.md", 5, dead(), Fix::None);
        other.link.url = "https://other.test/".into();
        let findings = [
            finding("b.md", 4, permanent(), applied()),
            finding("a.md", 9, permanent(), applied()),
            finding("a.md", 2, permanent(), applied()),
            finding(
                "a.md",
                3,
                permanent(),
                Fix::Proposed("https://p.test/".into()),
            ),
            finding(
                "a.md",
                6,
                dead(),
                manual(ManualReason::Archive(NoFix::Deadline)),
            ),
            other,
            finding("a.md", 7, Verdict::Blocked { status: 403 }, Fix::None),
            finding("a.md", 8, Verdict::Ok { chain: vec![] }, Fix::None),
        ];
        let out = render_markdown(&findings, &summary());
        let fixed =
            "<https://old.test/x#frag> | <https://new.test/x#frag> | moved permanently (301 200) |";
        let expected = format!(
            "## Fixed: 1 link in 3 places\n\n\
             | Page | Lines | Old link | New link | Why |\n\
             |---|---|---|---|---|\n\
             | a.md | 2, 9 | {fixed}\n\
             | b.md | 4 | {fixed}\n\n\
             ## Fixes to apply: 1 link in 1 place\n\n\
             Run `rotguard --apply` to write them.\n\n\
             | Page | Lines | Old link | New link | Why |\n\
             |---|---|---|---|---|\n\
             | a.md | 3 | <https://old.test/x#frag> | <https://p.test/> | moved permanently (301 200) |\n\n\
             ## Needs a hand fix: 1 link in 1 place\n\n\
             | Page | Lines | Link | Problem | Why rotguard couldn't fix it |\n\
             |---|---|---|---|---|\n\
             | a.md | 6 | <https://old.test/x#frag> | dead (404) | the run ran out of time looking for an archived copy |\n\n\
             <details><summary>2 more links to know about, no action needed</summary>\n\n\
             | Page | Lines | Link | What rotguard saw |\n\
             |---|---|---|---|\n\
             | a.md | 5 | <https://other.test/> | dead (404) |\n\
             | a.md | 7 | <https://old.test/x#frag> | blocked by a bot wall (403) |\n\n\
             </details>\n\n\
             4 links checked: 1 ok, 1 moved permanently, 1 dead, 1 suspect."
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn markdown_suspect_fix_needs_a_hand() {
        let suspect = Verdict::Suspect {
            chain: vec![
                hop("https://old.test/x", 301),
                hop("https://old.test/", 200),
            ],
            reason: "redirect to root".into(),
        };
        let fix = Fix::Proposed("https://web.test/s".into());
        let out = render_markdown(&[finding("a.md", 1, suspect, fix)], &summary());
        assert!(
            out.contains("## Needs a hand fix: 1 link in 1 place"),
            "{out}"
        );
        assert!(
            out.contains(
                "| a.md | 1 | <https://old.test/x#frag> | suspect (redirect to root) | \
                 rotguard doesn't rewrite suspect links; archived copy: <https://web.test/s> |"
            ),
            "{out}"
        );
        assert!(!out.contains("Fixes to apply"), "{out}");
    }

    #[test]
    fn markdown_wording_and_counts() {
        let reasons = [
            (
                ManualReason::Archive(NoFix::NoCapture),
                "no archived copy in the Wayback Machine",
            ),
            (
                ManualReason::Archive(NoFix::Unreachable),
                "the Wayback Machine didn't answer",
            ),
            (
                ManualReason::Archive(NoFix::Deadline),
                "the run ran out of time looking for an archived copy",
            ),
            (
                ManualReason::Unwritable(Unwritable::NotVerbatim),
                "the file writes the URL differently from the link",
            ),
            (
                ManualReason::Unwritable(Unwritable::CannotQuote),
                "the new URL can't be written inside this link",
            ),
            (
                ManualReason::ChangedDuringRun,
                "the file changed while rotguard ran",
            ),
        ];
        for (reason, text) in reasons {
            let out = render_markdown(
                &[finding("a.md", 1, dead(), Fix::Manual(reason))],
                &summary(),
            );
            assert!(
                out.contains(&format!("| dead (404) | {text} |")),
                "{text}\n{out}"
            );
        }
        let verdicts = [
            (
                Verdict::Temporary {
                    chain: vec![hop("https://old.test/x", 302), hop("https://t.test/y", 200)],
                    final_url: Url::parse("https://t.test/y").unwrap(),
                },
                "moved temporarily (302 200) to <https://t.test/y>",
            ),
            (
                Verdict::Suspect {
                    chain: vec![],
                    reason: "redirect to root".into(),
                },
                "suspect (redirect to root)",
            ),
            (
                Verdict::Unverifiable {
                    reason: "tls".into(),
                },
                "could not check (tls)",
            ),
        ];
        for (verdict, text) in verdicts {
            let out = render_markdown(&[finding("a.md", 1, verdict, Fix::None)], &summary());
            assert!(out.contains(&format!("| {text} |")), "{text}\n{out}");
        }
        let all = Summary {
            checked: 9,
            ok: 1,
            permanent: 1,
            temporary: 1,
            dead: 1,
            suspect: 1,
            unverifiable: 1,
            blocked: 1,
            ignored: 1,
        };
        assert_eq!(
            md_counts(&all),
            "9 links checked: 1 ok, 1 moved permanently, 1 moved temporarily, 1 dead, \
             1 suspect, 1 could not check, 1 blocked, 1 ignored."
        );
        let clean = Summary {
            checked: 1,
            ok: 1,
            ..Summary::default()
        };
        assert_eq!(
            render_markdown(&[], &clean),
            "No broken links.\n\n1 link checked: 1 ok."
        );
    }

    #[test]
    fn markdown_escapes_text_from_the_site() {
        for c in "\\`*_[]<>()!|~&#".chars() {
            assert_eq!(md_escape(&format!("a{c}b")), format!("a\\{c}b"), "{c:?}");
        }
        assert_eq!(md_escape("a\nb\tc"), "a b c");
        assert_eq!(md_link("https://x.test/a?b=1"), "<https://x.test/a?b=1>");
        assert_eq!(md_link("http://x.test"), "<http://x.test>");
        let cases = [
            ("space", "https://x.test/a b", "https://x.test/a b"),
            ("lt", "https://x.test/<", "https://x.test/\\<"),
            ("gt", "https://x.test/>", "https://x.test/\\>"),
            ("pipe", "https://x.test/a|b", "https://x.test/a\\|b"),
            ("backtick", "https://x.test/`", "https://x.test/\\`"),
            ("backslash", "https://x.test/\\", "https://x.test/\\\\"),
            ("control", "https://x.test/\u{7}", "https://x.test/ "),
            ("scheme", "ftp://x.test/", "ftp://x.test/"),
        ];
        for (name, url, text) in cases {
            let got = md_link(url);
            assert!(!got.starts_with('<'), "{name}: {got}");
            assert_eq!(got, text, "{name}");
        }
    }
}
