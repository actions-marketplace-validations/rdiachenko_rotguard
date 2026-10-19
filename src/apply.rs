//! Writing fixes into site files.

use crate::extract::{Ctx, Link, Quote};

/// Decides whether `fix` can be written at the position of `link`.
///
/// # Errors
///
/// Returns the reason the fix needs a manual edit: "not verbatim" when the
/// source bytes differ from the link's URL, "cannot quote" when the fix cannot
/// be written inside the link's delimiters.
pub fn writable(fix: &str, link: &Link) -> Result<(), String> {
    if !link.verbatim {
        return Err("not verbatim".to_string());
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
        Err("cannot quote".to_string())
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
    depth == 0
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let quote = Err("cannot quote".to_string());
        let rows: &[(&str, Ctx, bool, Result<(), String>)] = &[
            (
                "https://a.test/n",
                Ctx::MdDest,
                false,
                Err("not verbatim".to_string()),
            ),
            ("https://a.test/n", Ctx::MdDest, true, ok.clone()),
            ("https://a.test/n", Ctx::MdText, true, ok.clone()),
            ("https://a.test/n", Ctx::Html, true, ok.clone()),
            ("https://a.test/n", single, true, ok.clone()),
            ("https://a.test/n", double, true, ok.clone()),
            ("https://a.test/n", bare, true, ok.clone()),
            ("https://a.test/it's", single, true, quote.clone()),
            ("https://a.test/it's", double, true, ok.clone()),
            ("https://a.test/a\"b", double, true, quote.clone()),
            ("https://a.test/a\"b", single, true, ok.clone()),
            ("https://a.test/it's", bare, true, ok.clone()),
            ("https://a.test/a b", Ctx::MdDest, true, quote.clone()),
            ("https://a.test/a\tb", Ctx::MdDest, true, quote.clone()),
            ("https://a.test/M/a)b", Ctx::MdDest, true, quote.clone()),
            ("https://a.test/M/a(b", Ctx::MdDest, true, quote.clone()),
            (")(", Ctx::MdDest, true, quote.clone()),
            ("https://a.test/a(b)c", Ctx::MdDest, true, ok.clone()),
            ("https://a.test/a\\)b", Ctx::MdDest, true, quote.clone()),
            ("https://a.test/a\\b", Ctx::MdDest, true, quote.clone()),
            ("https://a.test/a\\b", double, true, quote.clone()),
            ("https://a.test/a\\b", single, true, ok.clone()),
            ("https://a.test/a\nb", Ctx::MdDest, true, quote.clone()),
            (
                "https://a.test/it's",
                single,
                false,
                Err("not verbatim".to_string()),
            ),
            ("https://a.test/a<b", Ctx::MdDest, true, quote.clone()),
            ("https://a.test/a>b", Ctx::MdDest, true, quote.clone()),
            ("https://a.test/a b", Ctx::MdText, true, ok.clone()),
        ];
        for (fix, ctx, verbatim, expected) in rows {
            assert_eq!(
                &writable(fix, &link(*ctx, *verbatim)),
                expected,
                "fix {fix:?} in {ctx:?}, verbatim {verbatim}"
            );
        }
    }
}
