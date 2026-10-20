//! Formats the findings and the summary line printed on stdout.

use std::fmt::Write as _;

use crate::check::{Hop, Verdict};
use crate::{Finding, Fix};

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
    fn permanent_prints_chain_and_fix() {
        let f = finding(
            "a.md",
            3,
            permanent(),
            Fix::Proposed("https://new.test/x#frag".into()),
        );
        let out = render(&[f], &Summary::default());
        assert!(
            out.starts_with(
                "a.md:3  PERMANENT REDIRECT 301 200  https://old.test/x#frag\n    fix: https://new.test/x#frag\n"
            ),
            "{out}"
        );
    }

    #[test]
    fn applied_prints_fix() {
        let f = finding(
            "a.md",
            3,
            permanent(),
            Fix::Applied("https://new.test/x".into()),
        );
        assert!(render(&[f], &Summary::default()).contains("\n    fix: https://new.test/x\n"));
    }

    #[test]
    fn dead_prints_reason_and_manual_fix() {
        let f = finding(
            "a.md",
            1,
            dead(),
            Fix::Manual(ManualReason::Archive(NoFix::NoCapture)),
        );
        let out = render(&[f], &Summary::default());
        assert!(
            out.starts_with(
                "a.md:1  DEAD 404  https://old.test/x#frag\n    manual fix: no archived copy\n"
            ),
            "{out}"
        );
    }

    #[test]
    fn temporary_prints_final_without_fix() {
        let verdict = Verdict::Temporary {
            chain: vec![hop("https://old.test/x", 302), hop("https://t.test/y", 200)],
            final_url: Url::parse("https://t.test/y").unwrap(),
        };
        let out = render(
            &[finding("a.md", 1, verdict, Fix::None)],
            &Summary::default(),
        );
        assert!(
            out.starts_with(
                "a.md:1  TEMPORARY REDIRECT 302 200  https://old.test/x#frag\n    final: https://t.test/y\n"
            ),
            "{out}"
        );
        assert!(!out.contains("fix:"));
    }

    #[test]
    fn unverifiable_has_no_detail_line() {
        let verdict = Verdict::Unverifiable {
            reason: "deadline".into(),
        };
        let out = render(
            &[finding("a.md", 1, verdict, Fix::None)],
            &Summary::default(),
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "{out}");
        assert_eq!(
            lines[0],
            "a.md:1  UNVERIFIABLE deadline  https://old.test/x#frag"
        );
    }

    #[test]
    fn suspect_prints_reason_and_fix_or_manual() {
        let verdict = || Verdict::Suspect {
            chain: vec![
                hop("https://old.test/x", 301),
                hop("https://old.test/", 200),
            ],
            reason: "redirect to root".into(),
        };
        let with_fix = finding(
            "a.md",
            1,
            verdict(),
            Fix::Proposed("https://web.test/s".into()),
        );
        let out = render(&[with_fix], &Summary::default());
        assert!(
            out.starts_with(
                "a.md:1  SUSPECT redirect to root  https://old.test/x#frag\n    fix: https://web.test/s\n"
            ),
            "{out}"
        );
        let manual = finding(
            "a.md",
            1,
            verdict(),
            Fix::Manual(ManualReason::Archive(NoFix::NoCapture)),
        );
        assert!(
            render(&[manual], &Summary::default()).contains("\n    manual fix: no archived copy\n")
        );
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
