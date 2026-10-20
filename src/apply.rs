//! Writing fixes into site files.

use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::check::Verdict;
use crate::extract::{Ctx, Link, Quote};
use crate::{Finding, Fix, ManualReason};

/// Writes every proposed fix for permanent redirects and dead links into the site files.
///
/// Fixes that cannot be written become `Fix::Manual`; a file whose spans no longer hold their
/// original URL is left untouched and its fixes become `Manual(ChangedDuringRun)`. Fixes
/// that were written become `Fix::Applied`. Each file is replaced atomically.
///
/// # Errors
///
/// Returns the I/O error from reading, writing or renaming a file. The original file is
/// left as it was.
pub fn apply(site_dir: &Path, findings: &mut [Finding]) -> io::Result<()> {
    let mut by_file: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
    for (index, finding) in findings.iter().enumerate() {
        let applicable = matches!(
            finding.verdict,
            Verdict::Permanent { .. } | Verdict::Dead { .. }
        ) && matches!(finding.fix, Fix::Proposed(_));
        if applicable {
            by_file.entry(finding.file.clone()).or_default().push(index);
        }
    }
    for (file, indices) in by_file {
        apply_file(&site_dir.join(file), findings, indices)?;
    }
    Ok(())
}

fn apply_file(path: &Path, findings: &mut [Finding], indices: Vec<usize>) -> io::Result<()> {
    let mut edits = Vec::new();
    for index in indices {
        let finding = &mut findings[index];
        let Fix::Proposed(fix) = &finding.fix else {
            continue;
        };
        match writable(fix, &finding.link) {
            Ok(()) => edits.push(index),
            Err(reason) => finding.fix = Fix::Manual(ManualReason::Unwritable(reason)),
        }
    }
    if edits.is_empty() {
        return Ok(());
    }

    let mut text = std::fs::read_to_string(path)?;
    // Last span first, so earlier offsets stay valid while replacing.
    edits.sort_by_key(|&index| std::cmp::Reverse(findings[index].link.span.start));
    let unchanged = edits.iter().all(|&index| {
        let link = &findings[index].link;
        text.get(link.span.clone()) == Some(link.url.as_str())
    });
    let overlapping = edits.windows(2).any(|pair| {
        let (later, earlier) = (&findings[pair[0]].link.span, &findings[pair[1]].link.span);
        later != earlier && earlier.end > later.start
    });
    if !unchanged || overlapping {
        for index in edits {
            findings[index].fix = Fix::Manual(ManualReason::ChangedDuringRun);
        }
        return Ok(());
    }

    let mut done: Option<std::ops::Range<usize>> = None;
    for &index in &edits {
        let finding = &findings[index];
        let Fix::Proposed(fix) = &finding.fix else {
            continue;
        };
        // The same occurrence listed twice is replaced once.
        if done.as_ref() != Some(&finding.link.span) {
            text.replace_range(finding.link.span.clone(), fix);
            done = Some(finding.link.span.clone());
        }
    }
    write_atomic(path, &text)?;
    for index in edits {
        if let Fix::Proposed(fix) = &findings[index].fix {
            findings[index].fix = Fix::Applied(fix.clone());
        }
    }
    Ok(())
}

/// Replaces `path` with `text` through `<path>.rotguard.tmp` in the same directory.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let mut name: OsString = path.file_name().unwrap_or_default().to_owned();
    name.push(".rotguard.tmp");
    let tmp = path.with_file_name(name);
    let result = std::fs::write(&tmp, text)
        .and_then(|()| std::fs::set_permissions(&tmp, std::fs::metadata(path)?.permissions()))
        .and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        // Best effort: the write error is the one worth reporting.
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Why a fix cannot be written at a link's position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unwritable {
    /// The source bytes differ from the link's URL.
    NotVerbatim,
    /// The fix cannot be written inside the link's delimiters.
    CannotQuote,
}

impl fmt::Display for Unwritable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotVerbatim => "not verbatim",
            Self::CannotQuote => "cannot quote",
        })
    }
}

impl Error for Unwritable {}

/// Decides whether `fix` can be written at the position of `link`.
///
/// # Errors
///
/// Returns [`Unwritable::NotVerbatim`] when the source bytes differ from the link's URL,
/// [`Unwritable::CannotQuote`] when the fix cannot be written inside the link's delimiters.
pub fn writable(fix: &str, link: &Link) -> Result<(), Unwritable> {
    if !link.verbatim {
        return Err(Unwritable::NotVerbatim);
    }
    let quotable = match link.ctx {
        Ctx::FrontMatter {
            quote: Quote::Single,
        } => !fix.contains('\''),
        Ctx::FrontMatter {
            quote: Quote::Double,
        } => !fix.contains(['"', '\\']),
        Ctx::MdDest => md_dest_ok(fix),
        Ctx::FrontMatter { quote: Quote::Bare } | Ctx::MdText | Ctx::Html => true,
    };
    if quotable {
        Ok(())
    } else {
        Err(Unwritable::CannotQuote)
    }
}

/// Whether `fix` is a valid CommonMark link destination without angle brackets.
fn md_dest_ok(fix: &str) -> bool {
    let mut depth = 0u32;
    for c in fix.chars() {
        match c {
            c if c.is_whitespace() => return false,
            '<' | '>' | '\\' => return false,
            '(' => depth += 1,
            ')' => {
                let Some(next) = depth.checked_sub(1) else {
                    return false;
                };
                depth = next;
            }
            _ => {}
        }
    }
    depth == 0 && !has_char_reference(fix)
}

/// Whether `s` holds an HTML character reference, which CommonMark decodes in a destination.
fn has_char_reference(s: &str) -> bool {
    s.match_indices('&').any(|(i, _)| {
        let rest = &s[i + 1..];
        let (digits, valid): (&str, fn(char) -> bool) = if let Some(num) = rest.strip_prefix('#') {
            match num.strip_prefix(['x', 'X']) {
                Some(hex) => (hex, |c| c.is_ascii_hexdigit()),
                None => (num, |c| c.is_ascii_digit()),
            }
        } else if rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
            (rest, |c| c.is_ascii_alphanumeric())
        } else {
            return false;
        };
        let len = digits.find(|c| !valid(c)).unwrap_or(digits.len());
        len > 0 && digits[len..].starts_with(';')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::Verdict;
    use url::Url;

    fn link(ctx: Ctx, verbatim: bool) -> Link {
        Link {
            url: "https://example.test/old".to_string(),
            line: 1,
            span: 0..24,
            ctx,
            verbatim,
        }
    }

    #[test]
    fn decides_writability() {
        let single = Ctx::FrontMatter {
            quote: Quote::Single,
        };
        let double = Ctx::FrontMatter {
            quote: Quote::Double,
        };
        let bare = Ctx::FrontMatter { quote: Quote::Bare };
        let ok = Ok(());
        let quote = Err(Unwritable::CannotQuote);
        let rows: &[(&str, Ctx, bool, Result<(), Unwritable>)] = &[
            (
                "https://a.test/n",
                Ctx::MdDest,
                false,
                Err(Unwritable::NotVerbatim),
            ),
            ("https://a.test/n", Ctx::MdDest, true, ok),
            ("https://a.test/n", Ctx::MdText, true, ok),
            ("https://a.test/n", Ctx::Html, true, ok),
            ("https://a.test/n", single, true, ok),
            ("https://a.test/n", double, true, ok),
            ("https://a.test/n", bare, true, ok),
            ("https://a.test/it's", single, true, quote),
            ("https://a.test/it's", double, true, ok),
            ("https://a.test/a\"b", double, true, quote),
            ("https://a.test/a\"b", single, true, ok),
            ("https://a.test/it's", bare, true, ok),
            ("https://a.test/a b", Ctx::MdDest, true, quote),
            ("https://a.test/a\tb", Ctx::MdDest, true, quote),
            ("https://a.test/M/a)b", Ctx::MdDest, true, quote),
            ("https://a.test/M/a(b", Ctx::MdDest, true, quote),
            (")(", Ctx::MdDest, true, quote),
            ("https://a.test/a(b)c", Ctx::MdDest, true, ok),
            ("https://a.test/a\\)b", Ctx::MdDest, true, quote),
            ("https://a.test/a\\b", Ctx::MdDest, true, quote),
            ("https://a.test/a\\b", double, true, quote),
            ("https://a.test/a\\b", single, true, ok),
            ("https://a.test/a\nb", Ctx::MdDest, true, quote),
            (
                "https://a.test/it's",
                single,
                false,
                Err(Unwritable::NotVerbatim),
            ),
            ("https://a.test/a<b", Ctx::MdDest, true, quote),
            ("https://a.test/a>b", Ctx::MdDest, true, quote),
            ("https://a.test/a b", Ctx::MdText, true, ok),
            ("https://m.test/n?x=&copy;", Ctx::MdDest, true, quote),
            ("https://m.test/n?x=&#38;", Ctx::MdDest, true, quote),
            ("https://m.test/n?x=&#x26;", Ctx::MdDest, true, quote),
            ("https://m.test/n?a=1&b=2", Ctx::MdDest, true, ok),
            ("https://m.test/n?x=&copy;", Ctx::MdText, true, ok),
        ];
        for (fix, ctx, verbatim, expected) in rows {
            assert_eq!(
                &writable(fix, &link(*ctx, *verbatim)),
                expected,
                "fix {fix:?} in {ctx:?}, verbatim {verbatim}"
            );
        }
    }

    const OLD_A: &str = "https://a.test/old";
    const OLD_B: &str = "https://b.test/gone";

    fn finding(text: &str, url: &str, fix: &str, verdict: Verdict, ctx: Ctx) -> Finding {
        let start = text.find(url).unwrap();
        Finding {
            file: PathBuf::from("post.md"),
            link: Link {
                url: url.to_string(),
                line: 1,
                span: start..start + url.len(),
                ctx,
                verbatim: true,
            },
            verdict,
            fix: Fix::Proposed(fix.to_string()),
        }
    }

    fn permanent() -> Verdict {
        Verdict::Permanent {
            chain: Vec::new(),
            fix: Url::parse("https://a.test/new").unwrap(),
        }
    }

    fn dead() -> Verdict {
        Verdict::Dead {
            reason: "404".to_string(),
        }
    }

    fn site(text: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("post.md"), text).unwrap();
        dir
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn replaces_two_spans_and_nothing_else() {
        let text = format!("one [a]({OLD_A}) two\n[b]({OLD_B}) three {OLD_A}\n");
        let dir = site(&text);
        let mut findings = vec![
            finding(&text, OLD_A, "https://a.test/new", permanent(), Ctx::MdDest),
            finding(&text, OLD_B, "https://web.test/snap", dead(), Ctx::MdDest),
        ];
        apply(dir.path(), &mut findings).unwrap();
        let expected = text
            .replacen(OLD_A, "https://a.test/new", 1)
            .replace(OLD_B, "https://web.test/snap");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("post.md")).unwrap(),
            expected
        );
        assert_eq!(
            findings[0].fix,
            Fix::Applied("https://a.test/new".to_string())
        );
        assert_eq!(
            findings[1].fix,
            Fix::Applied("https://web.test/snap".to_string())
        );
        assert_eq!(leftovers(dir.path()), ["post.md"]);
    }

    #[test]
    fn yaml_front_matter_changes_only_the_real_value() {
        let text =
            "---\nnote: https://a.test/old\n  is worth a read\nsource: https://a.test/old\n---\n";
        let dir = site(text);
        let page = crate::extract::extract(
            Path::new("post.md"),
            text,
            &crate::config::parse("site_url = \"https://site.test\"").unwrap(),
        );
        assert_eq!(page.links.len(), 1);
        let mut findings = vec![Finding {
            file: PathBuf::from("post.md"),
            link: page.links[0].clone(),
            verdict: permanent(),
            fix: Fix::Proposed("https://a.test/new".to_string()),
        }];
        apply(dir.path(), &mut findings).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("post.md")).unwrap(),
            "---\nnote: https://a.test/old\n  is worth a read\nsource: https://a.test/new\n---\n"
        );
    }

    #[test]
    fn changed_span_leaves_file_untouched() {
        let text = format!("[a]({OLD_A})\n[b]({OLD_B})\n");
        let dir = site(&text);
        let mut findings = vec![
            finding(&text, OLD_A, "https://a.test/new", permanent(), Ctx::MdDest),
            finding(&text, OLD_B, "https://web.test/snap", dead(), Ctx::MdDest),
        ];
        let rewritten = text.replace(OLD_B, "https://b.test/other");
        std::fs::write(dir.path().join("post.md"), &rewritten).unwrap();
        apply(dir.path(), &mut findings).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("post.md")).unwrap(),
            rewritten
        );
        let changed = Fix::Manual(ManualReason::ChangedDuringRun);
        assert_eq!(findings[0].fix, changed);
        assert_eq!(findings[1].fix, changed);
        assert_eq!(leftovers(dir.path()), ["post.md"]);
    }

    #[test]
    fn span_past_end_of_file_counts_as_changed() {
        let text = format!("[a]({OLD_A})\n");
        let dir = site(&text);
        let mut findings = vec![finding(
            &text,
            OLD_A,
            "https://a.test/new",
            permanent(),
            Ctx::MdDest,
        )];
        std::fs::write(dir.path().join("post.md"), "short").unwrap();
        apply(dir.path(), &mut findings).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("post.md")).unwrap(),
            "short"
        );
        assert_eq!(findings[0].fix, Fix::Manual(ManualReason::ChangedDuringRun));
    }

    #[test]
    fn unwritable_finding_is_set_aside_and_the_other_still_applies() {
        let text = format!("[a]({OLD_A})\n[b]({OLD_B})\n");
        let dir = site(&text);
        let mut findings = vec![
            finding(&text, OLD_A, "https://a.test/a)b", permanent(), Ctx::MdDest),
            finding(&text, OLD_B, "https://web.test/snap", dead(), Ctx::MdDest),
        ];
        // The set-aside finding must not make the file count as changed, even if its span moved.
        findings[0].link.span = 0..1;
        apply(dir.path(), &mut findings).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("post.md")).unwrap(),
            text.replace(OLD_B, "https://web.test/snap")
        );
        assert_eq!(
            findings[0].fix,
            Fix::Manual(ManualReason::Unwritable(Unwritable::CannotQuote))
        );
        assert_eq!(
            findings[1].fix,
            Fix::Applied("https://web.test/snap".to_string())
        );
        assert_eq!(leftovers(dir.path()), ["post.md"]);
    }

    #[test]
    fn suspects_and_other_fixes_are_never_applied() {
        let text = format!("[a]({OLD_A})\n");
        let dir = site(&text);
        let suspect = Verdict::Suspect {
            chain: Vec::new(),
            reason: "soft 404".to_string(),
        };
        let mut findings = vec![finding(
            &text,
            OLD_A,
            "https://web.test/snap",
            suspect,
            Ctx::MdDest,
        )];
        apply(dir.path(), &mut findings).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("post.md")).unwrap(),
            text
        );
        assert_eq!(
            findings[0].fix,
            Fix::Proposed("https://web.test/snap".to_string())
        );
    }

    #[test]
    fn unreadable_file_is_an_error_and_leaves_no_temp_file() {
        let text = format!("[a]({OLD_A})\n");
        let dir = tempfile::tempdir().unwrap();
        let mut findings = vec![finding(
            &text,
            OLD_A,
            "https://a.test/new",
            permanent(),
            Ctx::MdDest,
        )];
        assert!(apply(dir.path(), &mut findings).is_err());
        assert!(leftovers(dir.path()).is_empty());
    }
}
