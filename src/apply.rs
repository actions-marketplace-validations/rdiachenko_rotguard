//! Writing fixes into site files.

use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::check::Verdict;
use crate::extract::{Ctx, Link, Quote};
use crate::{Finding, Fix, ManualReason};

/// A file that `apply` could not write.
#[derive(Debug)]
pub struct ApplyError {
    /// The file, relative to the site directory.
    pub file: PathBuf,
    /// The I/O error from reading, writing or renaming it.
    pub source: io::Error,
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file.display())
    }
}

impl Error for ApplyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// Writes every proposed fix for permanent redirects and dead links into the site files.
///
/// Fixes that cannot be written become `Fix::Manual`; a file whose spans no longer hold their
/// original URL, or that was deleted or is no longer UTF-8, is left untouched and its fixes
/// become `Manual(ChangedDuringRun)`. Fixes that were written become `Fix::Applied`. Each file
/// is replaced atomically.
///
/// Every file is tried. Other I/O errors from reading, writing or renaming a file are returned
/// per file; that file is left as it was and its findings keep `Fix::Proposed`.
#[must_use]
pub fn apply(site_dir: &Path, findings: &mut [Finding]) -> Vec<ApplyError> {
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
    let mut errors = Vec::new();
    for (file, indices) in by_file {
        if let Err(source) = apply_file(&site_dir.join(&file), findings, indices) {
            errors.push(ApplyError { file, source });
        }
    }
    errors
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

    let mut text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // Deleted or rewritten as non-UTF-8 since the scan: same as an edited file.
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidData
            ) =>
        {
            for index in edits {
                findings[index].fix = Fix::Manual(ManualReason::ChangedDuringRun);
            }
            return Ok(());
        }
        Err(error) => return Err(error),
    };
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
    // `create_new` fails on any existing path, a planted symlink included, so
    // a failed open means the path isn't ours and must stay.
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&tmp) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "{} already exists; remove it and run again",
                    tmp.file_name().unwrap_or_default().to_string_lossy()
                ),
            ));
        }
        Err(error) => return Err(error),
    };
    let result = file
        .write_all(text.as_bytes())
        .and_then(|()| file.set_permissions(std::fs::metadata(path)?.permissions()))
        .and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| std::fs::rename(&tmp, path));
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
/// Returns [`Unwritable::NotVerbatim`] when the source bytes differ from the link's URL or the
/// link is raw HTML or a bare URL (never written),
/// [`Unwritable::CannotQuote`] when the fix cannot be written inside the link's delimiters.
pub fn writable(fix: &str, link: &Link) -> Result<(), Unwritable> {
    if !link.verbatim {
        return Err(Unwritable::NotVerbatim);
    }
    let quotable = match link.ctx {
        Ctx::MdText | Ctx::Html => return Err(Unwritable::NotVerbatim),
        Ctx::FrontMatter {
            quote: Quote::Single,
        } => !fix.contains('\''),
        Ctx::FrontMatter {
            quote: Quote::Double,
        } => !fix.contains(['"', '\\']),
        Ctx::MdDest => md_dest_ok(fix),
        Ctx::FrontMatter { quote: Quote::Bare } => true,
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
            (
                "https://a.test/n",
                Ctx::MdText,
                true,
                Err(Unwritable::NotVerbatim),
            ),
            (
                "https://a.test/n",
                Ctx::Html,
                true,
                Err(Unwritable::NotVerbatim),
            ),
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
            (
                "https://a.test/a b",
                Ctx::MdText,
                true,
                Err(Unwritable::NotVerbatim),
            ),
            ("https://m.test/n?x=&copy;", Ctx::MdDest, true, quote),
            ("https://m.test/n?x=&#38;", Ctx::MdDest, true, quote),
            ("https://m.test/n?x=&#x26;", Ctx::MdDest, true, quote),
            ("https://m.test/n?a=1&b=2", Ctx::MdDest, true, ok),
            (
                "https://m.test/n?x=&copy;",
                Ctx::MdText,
                true,
                Err(Unwritable::NotVerbatim),
            ),
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
        // The first fix is longer and the second shorter than their originals, with CRLF
        // line ends and multibyte text before each span: editing first-to-last would shift
        // the second span and corrupt it.
        const NEW_A: &str = "https://a.test/a-much-longer-replacement";
        const NEW_B: &str = "https://w.test/s";
        let text = format!(
            "caf\u{e9} \u{2014} [a]({OLD_A}) two\r\n\u{1f980} \u{2014} [b]({OLD_B}) three {OLD_A}\r\n"
        );
        let dir = site(&text);
        let mut findings = vec![
            finding(&text, OLD_A, NEW_A, permanent(), Ctx::MdDest),
            finding(&text, OLD_B, NEW_B, dead(), Ctx::MdDest),
        ];
        assert!(apply(dir.path(), &mut findings).is_empty());
        let expected = format!(
            "caf\u{e9} \u{2014} [a]({NEW_A}) two\r\n\u{1f980} \u{2014} [b]({NEW_B}) three {OLD_A}\r\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("post.md")).unwrap(),
            expected
        );
        assert_eq!(findings[0].fix, Fix::Applied(NEW_A.to_string()));
        assert_eq!(findings[1].fix, Fix::Applied(NEW_B.to_string()));
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
        assert!(apply(dir.path(), &mut findings).is_empty());
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
        assert!(apply(dir.path(), &mut findings).is_empty());
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
        assert!(apply(dir.path(), &mut findings).is_empty());
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
        assert!(apply(dir.path(), &mut findings).is_empty());
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
        assert!(apply(dir.path(), &mut findings).is_empty());
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
        // A directory in place of the file: a read error that is neither NotFound nor InvalidData.
        std::fs::create_dir(dir.path().join("post.md")).unwrap();
        let mut findings = vec![finding(
            &text,
            OLD_A,
            "https://a.test/new",
            permanent(),
            Ctx::MdDest,
        )];
        let errors = apply(dir.path(), &mut findings);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, Path::new("post.md"));
        assert_eq!(leftovers(dir.path()), ["post.md"]);
    }

    #[test]
    fn one_unreadable_file_does_not_stop_the_others() {
        let text = format!("[a]({OLD_A})\n");
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("a.md")).unwrap();
        std::fs::write(dir.path().join("b.md"), &text).unwrap();
        let mut findings = vec![
            finding(&text, OLD_A, "https://a.test/new", permanent(), Ctx::MdDest),
            finding(&text, OLD_A, "https://a.test/new", permanent(), Ctx::MdDest),
        ];
        findings[0].file = PathBuf::from("a.md");
        findings[1].file = PathBuf::from("b.md");
        let errors = apply(dir.path(), &mut findings);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("b.md")).unwrap(),
            text.replace(OLD_A, "https://a.test/new")
        );
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, Path::new("a.md"));
        assert_eq!(errors[0].to_string(), "a.md");
        assert!(errors[0].source().is_some());
        assert_eq!(
            findings[0].fix,
            Fix::Proposed("https://a.test/new".to_string())
        );
        assert_eq!(
            findings[1].fix,
            Fix::Applied("https://a.test/new".to_string())
        );
    }

    #[test]
    fn deleted_file_counts_as_changed() {
        let text = format!("[a]({OLD_A})\n");
        let dir = site(&text);
        let mut findings = vec![finding(
            &text,
            OLD_A,
            "https://a.test/new",
            permanent(),
            Ctx::MdDest,
        )];
        std::fs::remove_file(dir.path().join("post.md")).unwrap();
        assert!(apply(dir.path(), &mut findings).is_empty());
        assert_eq!(findings[0].fix, Fix::Manual(ManualReason::ChangedDuringRun));
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn non_utf8_file_counts_as_changed() {
        let text = format!("[a]({OLD_A})\n");
        let dir = site(&text);
        let mut findings = vec![finding(
            &text,
            OLD_A,
            "https://a.test/new",
            permanent(),
            Ctx::MdDest,
        )];
        let bytes = [0xff, 0xfe, 0x00];
        std::fs::write(dir.path().join("post.md"), bytes).unwrap();
        assert!(apply(dir.path(), &mut findings).is_empty());
        assert_eq!(std::fs::read(dir.path().join("post.md")).unwrap(), bytes);
        assert_eq!(findings[0].fix, Fix::Manual(ManualReason::ChangedDuringRun));
        assert_eq!(leftovers(dir.path()), ["post.md"]);
    }

    #[cfg(unix)]
    #[test]
    fn planted_temp_symlink_is_not_followed_or_removed() {
        let text = format!("[a]({OLD_A})\n");
        let dir = site(&text);
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "precious").unwrap();
        let tmp = dir.path().join("post.md.rotguard.tmp");
        std::os::unix::fs::symlink(&victim, &tmp).unwrap();
        let mut findings = vec![finding(
            &text,
            OLD_A,
            "https://a.test/new",
            permanent(),
            Ctx::MdDest,
        )];
        let errors = apply(dir.path(), &mut findings);
        assert_eq!(errors.len(), 1);
        let chain = crate::chain(&errors[0]);
        assert!(chain.contains("post.md.rotguard.tmp"), "{chain}");
        assert!(chain.contains("remove it and run again"), "{chain}");
        assert!(!chain.contains("not created by rotguard"), "{chain}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
        assert_eq!(std::fs::read_link(&tmp).unwrap(), victim);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("post.md")).unwrap(),
            text
        );
    }
}
