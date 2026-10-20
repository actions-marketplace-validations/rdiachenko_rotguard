//! Formats the findings and the summary line printed on stdout.

use std::fmt::{self, Write as _};

use serde::Serialize;
use url::Url;

use crate::check::{Hop, Verdict};
use crate::{Finding, Fix, Format};

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
    reason: Option<&'a str>,
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
                Verdict::Temporary { chain, final_url } => {
                    ("temporary", chain.as_slice(), None, Some(final_url))
                }
                Verdict::Dead { reason } => ("dead", [].as_slice(), Some(reason.as_str()), None),
                Verdict::Suspect { chain, reason } => {
                    ("suspect", chain.as_slice(), Some(reason.as_str()), None)
                }
                Verdict::Unverifiable { reason } => {
                    ("unverifiable", [].as_slice(), Some(reason.as_str()), None)
                }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::ManualReason;
    use crate::archive::NoFix;
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
}
