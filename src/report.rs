//! Formats the findings and the summary line printed on stdout.

use std::fmt::Write as _;

use crate::check::{Hop, Verdict};
use crate::{Finding, Fix, Format};

/// Counts of distinct URLs by outcome, computed by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Summary {
    pub checked: usize,
    pub ok: usize,
    pub permanent: usize,
    pub temporary: usize,
    pub dead: usize,
    pub suspect: usize,
    pub unverifiable: usize,
    pub ignored: usize,
}

/// Renders the report in `format`.
#[must_use]
pub fn render_as(format: Format, findings: &[Finding], summary: &Summary) -> String {
    match format {
        Format::Text => render(findings, summary),
    }
}

/// Renders one entry per non-ok finding, sorted by file then line, then the summary line.
///
/// Findings whose verdict is `Ok` are not reported. The result has no trailing newline.
#[must_use]
pub fn render(findings: &[Finding], summary: &Summary) -> String {
    let mut sorted: Vec<&Finding> = findings.iter().collect();
    sorted.sort_by(|a, b| (&a.file, a.link.line).cmp(&(&b.file, b.link.line)));

    let mut out = String::new();
    for finding in sorted {
        write_entry(&mut out, finding);
    }
    let Summary {
        checked,
        ok,
        permanent,
        temporary,
        dead,
        suspect,
        unverifiable,
        ignored,
    } = summary;
    write!(
        out,
        "checked {checked}: ok {ok}, permanent {permanent}, temporary {temporary}, \
         dead {dead}, suspect {suspect}, unverifiable {unverifiable}, ignored {ignored}"
    )
    .expect("writing to a String cannot fail");
    out
}

fn write_entry(out: &mut String, finding: &Finding) {
    let (class, detail) = match &finding.verdict {
        Verdict::Ok { .. } => return,
        Verdict::Permanent { chain, .. } => (
            format!("PERMANENT REDIRECT {}", statuses(chain)),
            fix_line(&finding.fix),
        ),
        Verdict::Temporary { chain, final_url } => (
            format!("TEMPORARY REDIRECT {}", statuses(chain)),
            Some(format!("final: {final_url}")),
        ),
        Verdict::Dead { reason } => (format!("DEAD {reason}"), fix_line(&finding.fix)),
        Verdict::Suspect { reason, .. } => (format!("SUSPECT {reason}"), fix_line(&finding.fix)),
        Verdict::Unverifiable { reason } => (format!("UNVERIFIABLE {reason}"), None),
    };
    let file = finding
        .file
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
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

fn fix_line(fix: &Fix) -> Option<String> {
    match fix {
        Fix::None => None,
        Fix::Proposed(url) | Fix::Applied(url) => Some(format!("fix: {url}")),
        Fix::Manual(reason) => Some(format!("manual fix: {reason}")),
    }
}

#[cfg(test)]
mod tests {
    use url::Url;

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
    fn summary_line_is_last_without_trailing_newline() {
        let out = render(&[finding("a.md", 1, dead(), Fix::None)], &summary());
        assert_eq!(
            out.lines().last().unwrap(),
            "checked 4: ok 1, permanent 1, temporary 0, dead 1, suspect 1, unverifiable 0, ignored 0"
        );
        assert!(!out.ends_with('\n'));
    }
}
