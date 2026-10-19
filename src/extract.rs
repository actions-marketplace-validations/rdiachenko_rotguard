//! Extraction of links and the page date from a content file's text.
//!
//! This task covers front matter only; the body is ignored.

use std::ops::Range;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use url::Url;

use crate::config::Config;

/// A content file with its date and the links found in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub path: PathBuf,
    pub date: Option<NaiveDate>,
    pub links: Vec<Link>,
}

/// One occurrence of an absolute http(s) URL in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    /// 1-based line of the span start.
    pub line: u32,
    /// File-absolute byte span of the URL in the source.
    pub span: Range<usize>,
    pub ctx: Ctx,
    /// Whether the source bytes at `span` equal `url`, so a fix can be written there.
    pub verbatim: bool,
}

/// Where a link was found, which decides how a fix is quoted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ctx {
    MdDest,
    MdText,
    Html,
    FrontMatter { quote: Quote },
}

/// The delimiter of a front-matter string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quote {
    Single,
    Double,
    Bare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Toml,
    Yaml,
}

/// A string token found by the lexer, as a range of the front-matter block.
struct Token {
    content: Range<usize>,
    quote: Quote,
}

/// Extracts the front-matter links and the page date from `text`.
#[must_use]
pub fn extract(path: &Path, text: &str, cfg: &Config) -> Page {
    let mut page = Page {
        path: path.to_path_buf(),
        date: None,
        links: Vec::new(),
    };
    let Some((format, block)) = front_matter(text) else {
        return page;
    };
    let block_text = &text[block.clone()];
    let Some(values) = parse_values(format, block_text, &cfg.date_key) else {
        // Unparseable front matter yields no links and no date; the body is unaffected.
        return page;
    };
    page.date = values.date;

    let mut remaining = values.urls;
    let tokens = match format {
        Format::Toml => lex_toml(block_text),
        Format::Yaml => lex_yaml(block_text),
    };
    let mut unmatched = Vec::new();
    for token in tokens {
        let content = &block_text[token.content.clone()];
        if let Some(i) = remaining.iter().position(|url| url == content) {
            remaining.swap_remove(i);
            let span = block.start + token.content.start..block.start + token.content.end;
            page.links
                .push(link(text, content, span, token.quote, true));
        } else {
            unmatched.push(token);
        }
    }
    // A value written with escapes equals the decoded form of its token, not the raw
    // bytes: it is checked at the token's position but never applied.
    for token in unmatched {
        if remaining.is_empty() {
            break;
        }
        let Some(decoded) = decode(format, &block_text[token.content.clone()], token.quote) else {
            continue;
        };
        if let Some(i) = remaining.iter().position(|url| *url == decoded) {
            remaining.swap_remove(i);
            let span = block.start + token.content.start..block.start + token.content.end;
            page.links
                .push(link(text, &decoded, span, token.quote, false));
        }
    }
    // Values still left were written in a form the lexer does not follow; they are
    // checked but never applied.
    for url in remaining {
        page.links
            .push(link(text, &url, block.clone(), Quote::Double, false));
    }
    page.links.sort_by_key(|l| l.span.start);
    page
}

fn link(text: &str, url: &str, span: Range<usize>, quote: Quote, verbatim: bool) -> Link {
    let line = text[..span.start].bytes().filter(|&b| b == b'\n').count() + 1;
    Link {
        url: url.to_owned(),
        line: u32::try_from(line).unwrap_or(u32::MAX),
        span,
        ctx: Ctx::FrontMatter { quote },
        verbatim,
    }
}

/// Finds the leading front-matter block: its format and the byte range between delimiters.
fn front_matter(text: &str) -> Option<(Format, Range<usize>)> {
    let (format, delim) = if text.starts_with("+++") {
        (Format::Toml, "+++")
    } else if text.starts_with("---") {
        (Format::Yaml, "---")
    } else {
        return None;
    };
    let mut lines = text.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim_end() != delim {
        return None;
    }
    let start = first.len();
    let mut offset = start;
    for line in lines {
        if line.trim_end() == delim {
            return Some((format, start..offset));
        }
        offset += line.len();
    }
    None
}

struct Values {
    /// Every absolute http(s) string value with a host, one entry per occurrence.
    urls: Vec<String>,
    date: Option<NaiveDate>,
}

fn parse_values(format: Format, block: &str, date_key: &str) -> Option<Values> {
    let mut urls = Vec::new();
    let date = match format {
        Format::Toml => {
            let table: toml::Table = toml::from_str(block).ok()?;
            collect_toml(&toml::Value::Table(table.clone()), &mut urls);
            table.get(date_key).and_then(toml_date)
        }
        Format::Yaml => {
            let value: serde_norway::Value = serde_norway::from_str(block).ok()?;
            collect_yaml(&value, &mut urls);
            value.get(date_key).and_then(yaml_date)
        }
    };
    Some(Values { urls, date })
}

/// Decodes the content of a quoted token the way the front-matter parser would.
fn decode(format: Format, content: &str, quote: Quote) -> Option<String> {
    match (format, quote) {
        (_, Quote::Bare) | (Format::Toml, Quote::Single) => None,
        (Format::Toml, Quote::Double) => {
            let table: toml::Table = toml::from_str(&format!("v = \"{content}\"")).ok()?;
            table.get("v")?.as_str().map(str::to_owned)
        }
        (Format::Yaml, Quote::Double) => serde_norway::from_str(&format!("\"{content}\"")).ok(),
        (Format::Yaml, Quote::Single) => serde_norway::from_str(&format!("'{content}'")).ok(),
    }
}

/// Whether `s` is an absolute http(s) URL with a host, exactly as written: the lenient
/// URL parser would otherwise repair padding, trailing text or a missing `//`.
fn keep(s: &str) -> bool {
    let has_scheme = ["http://", "https://"].iter().any(|prefix| {
        s.as_bytes()
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
    });
    has_scheme
        && !s.chars().any(char::is_whitespace)
        && Url::parse(s).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
}

fn collect_toml(value: &toml::Value, out: &mut Vec<String>) {
    match value {
        toml::Value::String(s) if keep(s) => out.push(s.clone()),
        toml::Value::Array(items) => items.iter().for_each(|v| collect_toml(v, out)),
        toml::Value::Table(table) => table.values().for_each(|v| collect_toml(v, out)),
        _ => {}
    }
}

fn collect_yaml(value: &serde_norway::Value, out: &mut Vec<String>) {
    match value {
        serde_norway::Value::String(s) if keep(s) => out.push(s.clone()),
        serde_norway::Value::Sequence(items) => items.iter().for_each(|v| collect_yaml(v, out)),
        serde_norway::Value::Mapping(map) => map.values().for_each(|v| collect_yaml(v, out)),
        serde_norway::Value::Tagged(tagged) => collect_yaml(&tagged.value, out),
        _ => {}
    }
}

fn string_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()
}

fn toml_date(value: &toml::Value) -> Option<NaiveDate> {
    match value {
        toml::Value::Datetime(dt) => {
            let date = dt.date?;
            NaiveDate::from_ymd_opt(i32::from(date.year), date.month.into(), date.day.into())
        }
        toml::Value::String(s) => string_date(s),
        _ => None,
    }
}

fn yaml_date(value: &serde_norway::Value) -> Option<NaiveDate> {
    value.as_str().and_then(string_date)
}

/// Finds the end of a quoted string starting after the opening quote at `from`.
/// Returns the index of the closing quote. `doubled` makes `''` an escaped quote.
fn closing_quote(s: &str, from: usize, quote: u8, backslash: bool, doubled: bool) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => return None,
            b'\\' if backslash => i += 1,
            b if b == quote => {
                if doubled && bytes.get(i + 1) == Some(&quote) {
                    i += 1;
                } else {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether the text after `from` makes the preceding string a key (`=` or `.` in TOML).
fn is_toml_key(s: &str, from: usize) -> bool {
    s[from..]
        .trim_start_matches([' ', '\t'])
        .starts_with(['=', '.'])
}

fn lex_toml(block: &str) -> Vec<Token> {
    let bytes = block.as_bytes();
    let mut tokens = Vec::new();
    // Nesting depth of multi-line arrays, so a `[` inside a value is not a table header.
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'#' => {
                i = block[i..].find('\n').map_or(bytes.len(), |n| i + n);
            }
            b'[' => {
                let line_start = block[..i].rfind('\n').map_or(0, |n| n + 1);
                if depth == 0 && block[line_start..i].trim().is_empty() {
                    // `[table]` / `[[array]]` header: its keys are never values.
                    i = block[i..].find('\n').map_or(bytes.len(), |n| i + n);
                } else {
                    depth += 1;
                }
            }
            b']' => depth = depth.saturating_sub(1),
            q @ (b'\'' | b'"') => {
                let triple = [q; 3];
                if bytes[i..].starts_with(&triple) {
                    // Multi-line strings never equal a single-line URL token; skip them whole.
                    let body = i + 3;
                    let end = block[body..]
                        .find(if q == b'"' { "\"\"\"" } else { "'''" })
                        .map_or(bytes.len(), |n| body + n + 3);
                    i = end;
                    continue;
                }
                let Some(end) = closing_quote(block, i + 1, q, q == b'"', false) else {
                    i += 1;
                    continue;
                };
                if !is_toml_key(block, end + 1) {
                    tokens.push(Token {
                        content: i + 1..end,
                        quote: if q == b'"' {
                            Quote::Double
                        } else {
                            Quote::Single
                        },
                    });
                }
                i = end;
            }
            _ => {}
        }
        i += 1;
    }
    tokens
}

fn lex_yaml(block: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut base = 0;
    // While set, lines indented deeper than this belong to the previous scalar
    // (block scalar body or plain/quoted continuation) and hold no tokens.
    let mut inside: Option<usize> = None;
    for line in block.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        let indent = text.len() - text.trim_start_matches(' ').len();
        if let Some(parent) = inside
            && (text.trim().is_empty() || indent > parent)
        {
            base += line.len();
            continue;
        }
        inside = lex_yaml_line(text, base, &mut tokens);
        base += line.len();
    }
    tokens
}

/// Whether `s` is a block scalar header: `|` or `>` with optional chomping/indent marks.
fn is_block_indicator(s: &str) -> bool {
    s.strip_prefix(['|', '>'])
        .is_some_and(|rest| rest.chars().all(|c| matches!(c, '+' | '-' | '0'..='9')))
}

/// Lexes one line. Returns the indent that following lines must exceed to still belong
/// to a scalar that began on this line.
fn lex_yaml_line(line: &str, base: usize, tokens: &mut Vec<Token>) -> Option<usize> {
    let bytes = line.as_bytes();
    let skip_spaces = |mut p: usize| {
        while p < bytes.len() && bytes[p] == b' ' {
            p += 1;
        }
        p
    };
    let indent = skip_spaces(0);
    let mut pos = indent;
    // Sequence indicators.
    while line[pos..].starts_with("- ") || &line[pos..] == "-" {
        pos = skip_spaces(pos + 1);
    }
    let first = pos;
    let mut had_key = false;
    loop {
        pos = skip_spaces(pos);
        if pos >= bytes.len() || bytes[pos] == b'#' {
            return None;
        }
        let parent = if had_key { first } else { indent };
        let c = bytes[pos];
        if c == b'\'' || c == b'"' {
            let Some(end) = closing_quote(line, pos + 1, c, c == b'"', c == b'\'') else {
                // A quoted string running over the line end continues below.
                return Some(parent);
            };
            let after = skip_spaces(end + 1);
            let is_key =
                bytes.get(after) == Some(&b':') && bytes.get(after + 1).is_none_or(|&b| b == b' ');
            if is_key {
                had_key = true;
                pos = after + 1;
                continue;
            }
            tokens.push(Token {
                content: base + pos + 1..base + end,
                quote: if c == b'"' {
                    Quote::Double
                } else {
                    Quote::Single
                },
            });
            return None;
        }
        // A comment starts at `#` preceded by a space or a tab.
        let stop = line[pos..]
            .match_indices('#')
            .find(|&(n, _)| n > 0 && matches!(bytes[pos + n - 1], b' ' | b'\t'))
            .map_or(bytes.len(), |(n, _)| pos + n);
        let segment = &line[pos..stop];
        let key_end = segment
            .find(": ")
            .or_else(|| segment.ends_with(':').then(|| segment.len() - 1));
        if let Some(k) = key_end {
            had_key = true;
            pos += k + 1;
            continue;
        }
        let trimmed = segment.trim_end();
        if !is_block_indicator(trimmed) {
            tokens.push(Token {
                content: base + pos..base + pos + trimmed.len(),
                quote: Quote::Bare,
            });
        }
        return Some(parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn cfg() -> Config {
        config::parse("site_url = \"https://site.test\"").expect("fixture config is valid")
    }

    fn run(text: &str) -> Page {
        extract(Path::new("post.md"), text, &cfg())
    }

    fn assert_slices(text: &str, page: &Page) {
        for l in &page.links {
            if l.verbatim {
                assert_eq!(&text[l.span.clone()], l.url);
            }
        }
    }

    #[test]
    fn toml_nested_array_of_tables_and_date() {
        let text = "+++\ntitle = \"T\"\ndate = 2024-03-05T10:00:00Z\n\n[[discussedOn]]\nurl = \"https://a.test/x\"\n\n[[discussedOn]]\nurl = 'https://b.test/y'\n+++\nbody https://ignored.test\n";
        let page = run(text);
        assert_eq!(page.date, NaiveDate::from_ymd_opt(2024, 3, 5));
        let got: Vec<_> = page
            .links
            .iter()
            .map(|l| (l.url.as_str(), l.line, l.ctx, l.verbatim))
            .collect();
        assert_eq!(
            got,
            [
                (
                    "https://a.test/x",
                    6,
                    Ctx::FrontMatter {
                        quote: Quote::Double
                    },
                    true
                ),
                (
                    "https://b.test/y",
                    9,
                    Ctx::FrontMatter {
                        quote: Quote::Single
                    },
                    true
                ),
            ]
        );
        assert_slices(text, &page);
    }

    #[test]
    fn toml_date_string() {
        let page = run("+++\ndate = \"2023-12-31T08:00:00+01:00\"\n+++\n");
        assert_eq!(page.date, NaiveDate::from_ymd_opt(2023, 12, 31));
    }

    #[test]
    fn toml_comment_and_prefix_do_not_match() {
        let text =
            "+++\n# old 'M/a'\nsource = 'M/a'  # also 'M/a'\nother = 'M/ab'\n\"M/a\" = 1\n+++\n";
        let text = text.replace("M/", "https://m.test/");
        let page = run(&text);
        let a: Vec<_> = page
            .links
            .iter()
            .filter(|l| l.url == "https://m.test/a")
            .collect();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].line, 3);
        let value = "source = '";
        assert_eq!(a[0].span.start, text.find(value).unwrap() + value.len());
        assert_eq!(page.links.len(), 2);
        assert_slices(&text, &page);
    }

    #[test]
    fn toml_table_header_keys_are_not_values() {
        for header in ["[\"https://a.test/x\"]", "[[\"https://a.test/x\"]]"] {
            let text = format!("+++\n{header}\nnote = \"https://a.test/x\"\n+++\n");
            let page = run(&text);
            assert_eq!(page.links.len(), 1, "{header}");
            assert_eq!(page.links[0].line, 3);
            assert!(page.links[0].verbatim);
            assert_slices(&text, &page);
        }
    }

    #[test]
    fn toml_multiline_array_values_are_lexed() {
        let text = "+++\nl = [\n  [\"https://a.test/x\"],\n  'https://b.test/y',\n]\n+++\n";
        let page = run(text);
        assert_eq!(page.links.len(), 2);
        assert!(page.links.iter().all(|l| l.verbatim));
        assert_slices(text, &page);
    }

    #[test]
    fn yaml_block_scalar_body_is_not_a_value() {
        let text = "---\ndesc: |-\n  https://a.test/x\nlink: https://a.test/x\n---\n";
        let page = run(text);
        // The block scalar is a real value too, but it is never applied.
        let verbatim: Vec<_> = page.links.iter().filter(|l| l.verbatim).collect();
        assert_eq!(verbatim.len(), 1);
        assert_eq!(verbatim[0].line, 4);
        assert_eq!(page.links.len(), 2);
        assert_slices(text, &page);
    }

    #[test]
    fn yaml_plain_continuation_line_is_not_a_value() {
        let text = "---\ndesc: intro\n  https://a.test/x\nlink: https://a.test/x\n---\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        assert_eq!(page.links[0].line, 4);
        assert!(page.links[0].verbatim);
        assert_slices(text, &page);
    }

    #[test]
    fn yaml_comment_after_tab_is_cut() {
        let text = "---\nu: https://a.test/x\t# c\n---\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        assert!(page.links[0].verbatim);
        assert_slices(text, &page);
    }

    #[test]
    fn repaired_urls_are_not_kept() {
        for value in [
            " https://a.test/x ",
            "https://a.test/ is great",
            "http:b.test",
            "https://a.test/x\n",
        ] {
            let text = format!("+++\nu = {value:?}\n+++\n");
            assert!(run(&text).links.is_empty(), "{value:?}");
        }
        let text = "---\nu: |\n  https://a.test/x\n---\n";
        assert!(run(text).links.is_empty());
        assert_eq!(run("+++\nu = \"HTTPS://A.test/x\"\n+++\n").links.len(), 1);
    }

    #[test]
    fn toml_escaped_value_is_not_verbatim() {
        let text = "+++\nu = \"https://e.test/a\\u0062\"\n+++\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        assert_eq!(page.links[0].url, "https://e.test/ab");
        assert!(!page.links[0].verbatim);
    }

    #[test]
    fn escaped_value_keeps_its_token_position() {
        let text = "+++\ntitle = \"T\"\nu = \"https://e.test/a\\u0062\"\n+++\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        let l = &page.links[0];
        assert_eq!((l.line, l.verbatim), (3, false));
        assert_eq!(
            l.ctx,
            Ctx::FrontMatter {
                quote: Quote::Double
            }
        );
        assert_eq!(&text[l.span.clone()], "https://e.test/a\\u0062");
    }

    #[test]
    fn toml_skips_non_http_values() {
        let page = run(
            "+++\na = \"mailto:someone\"\nb = \"/rel\"\nc = \"//h.test/x\"\nd = \"https://\"\n+++\n",
        );
        assert!(page.links.is_empty());
    }

    #[test]
    fn yaml_nested_values_and_date() {
        let text = "---\ntitle: T\ndate: 2024-03-05\ndiscussedOn:\n  - url: https://a.test/x\n  - \"https://b.test/y\"\n  - 'https://c.test/z' # note https://a.test/x\nnested:\n  deep:\n    link: https://d.test/w  # c\n---\nbody\n";
        let page = run(text);
        assert_eq!(page.date, NaiveDate::from_ymd_opt(2024, 3, 5));
        let got: Vec<_> = page
            .links
            .iter()
            .map(|l| (l.url.as_str(), l.line, l.ctx))
            .collect();
        let fm = |quote| Ctx::FrontMatter { quote };
        assert_eq!(
            got,
            [
                ("https://a.test/x", 5, fm(Quote::Bare)),
                ("https://b.test/y", 6, fm(Quote::Double)),
                ("https://c.test/z", 7, fm(Quote::Single)),
                ("https://d.test/w", 10, fm(Quote::Bare)),
            ]
        );
        assert!(page.links.iter().all(|l| l.verbatim));
        assert_slices(text, &page);
    }

    #[test]
    fn yaml_date_string() {
        let page = run("---\ndate: \"2022-01-02 10:00\"\n---\n");
        assert_eq!(page.date, NaiveDate::from_ymd_opt(2022, 1, 2));
    }

    #[test]
    fn yaml_escaped_value_is_not_verbatim() {
        let text = "---\nu: \"https://e.test/a\\x62\"\n---\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        assert_eq!(page.links[0].url, "https://e.test/ab");
        assert!(!page.links[0].verbatim);
    }

    #[test]
    fn no_front_matter_or_unclosed_yields_nothing() {
        assert!(run("# title\nhttps://a.test\n").links.is_empty());
        assert!(run("+++\nu = \"https://a.test\"\n").links.is_empty());
        assert_eq!(run("+++\nu = \n+++\n").links, []);
    }

    #[test]
    fn crlf_front_matter() {
        let text = "---\r\ndate: 2024-01-02\r\nu: https://a.test/x\r\n---\r\n";
        let page = run(text);
        assert_eq!(page.date, NaiveDate::from_ymd_opt(2024, 1, 2));
        assert_eq!(page.links.len(), 1);
        assert_slices(text, &page);
    }
}
