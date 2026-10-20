//! Extraction of links and the page date from a content file's text.
//!
//! Front matter values and markdown body links (inline links and images, reference
//! definitions, autolinks and `href`/`src` in raw HTML) and bare URLs in text are covered.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::NaiveDate;
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use regex::Regex;
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

/// Extracts the links and the page date from `text`.
#[must_use]
pub fn extract(path: &Path, text: &str, cfg: &Config) -> Page {
    let mut page = Page {
        path: path.to_path_buf(),
        date: None,
        links: Vec::new(),
    };
    let lines = Lines::new(text);
    let mut body_start = 0;
    if let Some((format, block)) = front_matter(text) {
        // The body starts after the line holding the closing delimiter.
        body_start = text[block.end..]
            .find('\n')
            .map_or(text.len(), |n| block.end + n + 1);
        front_matter_links(&mut page, text, &lines, cfg, format, &block);
    }
    body_links(&mut page, text, &lines, body_start);
    page.links.sort_by_key(|l| l.span.start);
    page
}

/// Byte offsets of every newline in a file, for turning offsets into line numbers.
struct Lines(Vec<usize>);

impl Lines {
    fn new(text: &str) -> Self {
        Self(text.match_indices('\n').map(|(i, _)| i).collect())
    }

    /// 1-based line of the byte `offset`.
    fn line_of(&self, offset: usize) -> u32 {
        let line = self.0.partition_point(|&n| n < offset) + 1;
        u32::try_from(line).unwrap_or(u32::MAX)
    }
}

fn front_matter_links(
    page: &mut Page,
    text: &str,
    lines: &Lines,
    cfg: &Config,
    format: Format,
    block: &Range<usize>,
) {
    let block_text = &text[block.clone()];
    let Some(values) = parse_values(format, block_text, &cfg.date_key) else {
        // Unparseable front matter yields no links and no date; the body is unaffected.
        return;
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
            page.links.push(link(
                lines,
                content,
                span,
                Ctx::FrontMatter { quote: token.quote },
                true,
            ));
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
            page.links.push(link(
                lines,
                &decoded,
                span,
                Ctx::FrontMatter { quote: token.quote },
                false,
            ));
        }
    }
    // Values still left were written in a form the lexer does not follow; they are
    // checked but never applied.
    for url in remaining {
        page.links.push(link(
            lines,
            &url,
            block.clone(),
            Ctx::FrontMatter {
                quote: Quote::Double,
            },
            false,
        ));
    }
}

fn link(lines: &Lines, url: &str, span: Range<usize>, ctx: Ctx, verbatim: bool) -> Link {
    Link {
        url: url.to_owned(),
        line: lines.line_of(span.start),
        span,
        ctx,
        verbatim,
    }
}

static BARE_URL: LazyLock<Regex> = LazyLock::new(|| {
    // The scheme may follow markdown delimiters such as `_`, `*` or `~`, so `\b` is not used.
    Regex::new(r#"(?i)(?:^|[^A-Za-z0-9])(https?://[^\s"'<>]+)"#).expect("regex is a literal")
});

static HTML_ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(?:href|src)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).expect("regex is a literal")
});

/// An open inline link, image or autolink while its events are consumed.
struct OpenLink {
    range: Range<usize>,
    link_type: LinkType,
    dest: String,
    /// End of the link text so far; an inline destination is searched for after it.
    text_end: usize,
}

/// Extracts the links of the markdown body that starts at `body_start`. The body is parsed
/// alone, so front matter never reaches the markdown parser.
fn body_links(page: &mut Page, text: &str, lines: &Lines, body_start: usize) {
    let body = &text[body_start..];
    let mut parser = Parser::new_ext(body, Options::empty()).into_offset_iter();
    let mut open: Vec<OpenLink> = Vec::new();
    // Where bare URLs are searched: text containers minus the ranges already covered.
    let mut containers: Vec<Range<usize>> = Vec::new();
    let mut covered: Vec<Range<usize>> = Vec::new();
    let mut container_start: Option<usize> = None;
    // The inline run of a tight list item, which sits outside any paragraph.
    let mut tight: Option<Range<usize>> = None;
    let mut in_code_block = false;
    for (event, range) in parser.by_ref() {
        let range = body_start + range.start..body_start + range.end;
        match &event {
            Event::Start(Tag::Paragraph | Tag::Heading { .. }) => {
                container_start = Some(range.start)
            }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_)) => {
                if let Some(start) = container_start.take() {
                    containers.push(start..range.end);
                }
            }
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => in_code_block = false,
            Event::End(TagEnd::Link | TagEnd::Image) => covered.push(range.clone()),
            Event::Code(_) | Event::Html(_) | Event::InlineHtml(_) => covered.push(range.clone()),
            _ => {}
        }
        // One container per inline run outside a paragraph or heading, closed at the next
        // block-level event.
        if container_start.is_none() && !in_code_block && is_inline(&event) {
            match &mut tight {
                Some(run) => run.end = run.end.max(range.end),
                None => tight = Some(range.clone()),
            }
        } else if let Some(run) = tight.take() {
            containers.push(run);
        }
        match event {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            })
            | Event::Start(Tag::Image {
                link_type,
                dest_url,
                ..
            }) => {
                open.push(OpenLink {
                    text_end: range.start + 1,
                    range,
                    link_type,
                    dest: dest_url.into_string(),
                });
                continue;
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                if let Some(done) = open.pop() {
                    push_dest(page, text, lines, &done);
                }
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                for caps in HTML_ATTR.captures_iter(&text[range.clone()]) {
                    let Some(value) = caps.get(1).or_else(|| caps.get(2)) else {
                        continue;
                    };
                    let raw = value.as_str();
                    let decoded = decode_entities(raw);
                    if keep(&decoded) {
                        let span = range.start + value.start()..range.start + value.end();
                        // Character references mean the raw bytes are not the URL.
                        page.links
                            .push(link(lines, &decoded, span, Ctx::Html, decoded == raw));
                    }
                }
            }
            _ => {}
        }
        if let Some(parent) = open.last_mut() {
            parent.text_end = range.end;
        }
    }
    // Reference definitions count once each, whether or not they are used.
    for (_, def) in parser.reference_definitions().iter() {
        if !keep(&def.dest) {
            continue;
        }
        let span = body_start + def.span.start..body_start + def.span.end;
        // The destination follows the label's `]:`.
        let from = text[span.clone()].find("]:").map_or(0, |n| n + 2);
        page.links
            .push(dest_link(text, lines, &def.dest, span, from));
    }
    containers.extend(tight);
    for container in containers {
        bare_links(page, text, lines, container, &covered);
    }
}

/// Records the bare URLs in `container`, skipping the `covered` sub-ranges.
fn bare_links(
    page: &mut Page,
    text: &str,
    lines: &Lines,
    container: Range<usize>,
    covered: &[Range<usize>],
) {
    let mut cuts: Vec<&Range<usize>> = covered
        .iter()
        .filter(|c| c.start < container.end && c.end > container.start)
        .collect();
    cuts.sort_by_key(|c| c.start);
    let mut from = container.start;
    for cut in cuts {
        if cut.start > from {
            scan_bare(page, text, lines, from..cut.start);
        }
        from = from.max(cut.end);
    }
    if from < container.end {
        scan_bare(page, text, lines, from..container.end);
    }
}

/// Whether `event` is part of an inline run (as opposed to a block-level event).
fn is_inline(event: &Event) -> bool {
    match event {
        Event::Text(_)
        | Event::Code(_)
        | Event::InlineHtml(_)
        | Event::SoftBreak
        | Event::HardBreak => true,
        Event::Start(tag) => matches!(
            tag,
            Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. }
        ),
        Event::End(tag) => matches!(
            tag,
            TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Link
                | TagEnd::Image
        ),
        _ => false,
    }
}

fn scan_bare(page: &mut Page, text: &str, lines: &Lines, segment: Range<usize>) {
    for caps in BARE_URL.captures_iter(&text[segment.clone()]) {
        let Some(m) = caps.get(1) else { continue };
        let raw = trim_url(m.as_str());
        let decoded = unescape(raw);
        if keep(&decoded) {
            let start = segment.start + m.start();
            page.links.push(link(
                lines,
                &decoded,
                start..start + raw.len(),
                Ctx::MdText,
                decoded == raw,
            ));
        }
    }
}

/// Decodes CommonMark backslash escapes and HTML character references.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut run = 0;
    let mut chars = raw.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '\\'
            && let Some(&(_, next)) = chars.peek()
            && next.is_ascii_punctuation()
        {
            out.push_str(&decode_entities(&raw[run..i]));
            out.push(next);
            chars.next();
            run = i + 1 + next.len_utf8();
        }
    }
    out.push_str(&decode_entities(&raw[run..]));
    out
}

/// Trims trailing punctuation and markdown delimiters, and a trailing `)` or `]` unless its
/// brackets balance.
fn trim_url(mut url: &str) -> &str {
    loop {
        let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '_', '~']);
        let trimmed = with_escape(url, trimmed);
        let stripped = strip_unbalanced(trimmed, ')', '(');
        let trimmed = with_escape(trimmed, stripped);
        let stripped = strip_unbalanced(trimmed, ']', '[');
        let trimmed = with_escape(trimmed, stripped);
        if trimmed.len() == url.len() {
            return url;
        }
        url = trimmed;
    }
}

/// After `before` was cut down to `after`, drops the `\` that escaped the first removed
/// character, so the URL never ends in half of an escape.
fn with_escape<'a>(before: &str, after: &'a str) -> &'a str {
    let slashes = after.bytes().rev().take_while(|&b| b == b'\\').count();
    if after.len() < before.len() && slashes % 2 == 1 {
        &after[..after.len() - 1]
    } else {
        after
    }
}

fn strip_unbalanced(s: &str, close: char, open: char) -> &str {
    match s.strip_suffix(close) {
        Some(rest) if s.matches(close).count() > s.matches(open).count() => rest,
        _ => s,
    }
}

/// Records the destination of a finished inline link, image or autolink.
fn push_dest(page: &mut Page, text: &str, lines: &Lines, done: &OpenLink) {
    if !keep(&done.dest) {
        return;
    }
    let from = match done.link_type {
        // The destination follows the `](` after the link text.
        LinkType::Inline => text[done.text_end..done.range.end]
            .find("](")
            .map_or(done.range.len(), |n| {
                done.text_end - done.range.start + n + 2
            }),
        LinkType::Autolink => 0,
        // Reference-style uses are skipped: their definition already counts.
        _ => return,
    };
    page.links
        .push(dest_link(text, lines, &done.dest, done.range.clone(), from));
}

/// Builds an `MdDest` link for `dest`. The raw destination token is read from `range` at
/// `from`; it is verbatim only when it equals `dest`, otherwise the whole `range` stands in
/// for the span.
fn dest_link(text: &str, lines: &Lines, dest: &str, range: Range<usize>, from: usize) -> Link {
    let base = range.start + from;
    let raw = text[range.clone()]
        .get(from..)
        .and_then(raw_destination)
        .filter(|raw| &text[base + raw.start..base + raw.end] == dest);
    match raw {
        Some(raw) => {
            let start = base + raw.start;
            link(lines, dest, start..start + dest.len(), Ctx::MdDest, true)
        }
        None => link(lines, dest, range, Ctx::MdDest, false),
    }
}

/// The range of the raw destination token at the start of `s` (after optional whitespace):
/// the content of `<…>`, or the run up to whitespace or the unbalanced closing `)`.
fn raw_destination(s: &str) -> Option<Range<usize>> {
    let bytes = s.as_bytes();
    let start = s.len() - s.trim_start().len();
    if bytes.get(start) == Some(&b'<') {
        let mut i = start + 1;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i += 1,
                b'>' => return Some(start + 1..i),
                b'\n' | b'<' => return None,
                _ => {}
            }
            i += 1;
        }
        return None;
    }
    let mut depth = 0_usize;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 1,
            b'(' => depth += 1,
            b')' if depth == 0 => break,
            b')' => depth -= 1,
            b if b.is_ascii_whitespace() => break,
            _ => {}
        }
        i += 1;
    }
    Some(start..i.min(bytes.len()))
}

/// Decodes the HTML character references `&amp; &lt; &gt; &quot; &#39; &apos;` and numeric
/// `&#NN;` / `&#xHH;`; anything else stays as written.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest.find(';').filter(|&end| end <= 10).and_then(|end| {
            let name = &rest[1..end];
            let ch = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => {
                    let num = name.strip_prefix('#')?;
                    let code = match num.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => num.parse().ok()?,
                    };
                    char::from_u32(code).filter(|&c| c != '\0')?
                }
            };
            Some((ch, end + 1))
        });
        match decoded {
            Some((ch, len)) => {
                out.push(ch);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Finds the leading front-matter block: its format and the byte range between delimiters.
fn front_matter(text: &str) -> Option<(Format, Range<usize>)> {
    // Only the first U+FEFF is a byte-order mark; it is not part of the
    // delimiter line, and the range stays an offset into the original text.
    let shift = if text.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let (format, block) = front_matter_unshifted(&text[shift..])?;
    Some((format, block.start + shift..block.end + shift))
}

/// Finds the front-matter block in `text`, which has no byte-order mark.
fn front_matter_unshifted(text: &str) -> Option<(Format, Range<usize>)> {
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
    // Index of the plain token the previous line pushed, which a continuation line
    // turns into part of a longer, non-URL scalar.
    let mut plain: Option<usize> = None;
    // Indent of a key whose value has not started: a plain scalar on the next line is
    // that key's value, so its continuation lines are measured against the key.
    let mut valueless: Option<usize> = None;
    for line in block.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        // A tab counts as a space here, as it does everywhere a line's leading
        // whitespace is read.
        let body = text.trim_start_matches([' ', '\t']);
        let indent = text.len() - body.len();
        if let Some(parent) = inside
            && (text.trim().is_empty() || indent > parent)
        {
            // A `#` line is a comment and leaves the value alone; any other
            // non-blank line continues the scalar, so the plain token is not a URL value.
            if !text.trim().is_empty()
                && !body.starts_with('#')
                && let Some(i) = plain.take()
            {
                tokens.remove(i);
            }
            base += line.len();
            continue;
        }
        if valueless.is_some() && (text.trim().is_empty() || body.starts_with('#')) {
            base += line.len();
            continue;
        }
        let before = tokens.len();
        let key_indent = valueless.take();
        inside = match lex_yaml_line(text, base, &mut tokens) {
            Line::Done => None,
            Line::Open(n) => Some(n),
            Line::KeyOnly(n) => {
                valueless = Some(n);
                None
            }
            Line::Scalar(n) => Some(key_indent.filter(|&k| indent > k).unwrap_or(n)),
        };
        plain = (tokens.len() > before && tokens[before].quote == Quote::Bare).then_some(before);
        base += line.len();
    }
    tokens
}

/// What a lexed YAML line leaves open for the lines below it.
enum Line {
    /// Nothing: the line's value, if any, ended on it.
    Done,
    /// A scalar that following lines continue while indented deeper than this.
    Open(usize),
    /// A key with no value, or only node properties, at this indent.
    KeyOnly(usize),
    /// Like `Open`, for a plain scalar that is the whole line (no key, no `- `).
    Scalar(usize),
}

/// Whether `s` is a block scalar header: `|` or `>` with optional chomping/indent marks.
fn is_block_indicator(s: &str) -> bool {
    s.strip_prefix(['|', '>'])
        .is_some_and(|rest| rest.chars().all(|c| matches!(c, '+' | '-' | '0'..='9')))
}

/// Lexes one line. Reports the indent that following lines must exceed to still belong
/// to a scalar that began on this line.
fn lex_yaml_line(line: &str, base: usize, tokens: &mut Vec<Token>) -> Line {
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
        // Node properties (anchors and tags) are not part of the value; a value that is
        // only properties introduces the nested block below it.
        while matches!(bytes.get(pos), Some(b'&' | b'!')) {
            // YAML separates a property from what follows by a space or a tab.
            let end = line[pos..]
                .find([' ', '\t'])
                .map_or(bytes.len(), |n| pos + n);
            pos = end;
            while matches!(bytes.get(pos), Some(b' ' | b'\t')) {
                pos += 1;
            }
        }
        if pos >= bytes.len() || bytes[pos] == b'#' {
            return if had_key {
                Line::KeyOnly(first)
            } else {
                Line::Done
            };
        }
        let parent = if had_key { first } else { indent };
        let c = bytes[pos];
        if c == b'\'' || c == b'"' {
            let Some(end) = closing_quote(line, pos + 1, c, c == b'"', c == b'\'') else {
                // A quoted string running over the line end continues below.
                return Line::Open(parent);
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
            return Line::Done;
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
        return if had_key || first != indent {
            Line::Open(parent)
        } else {
            Line::Scalar(parent)
        };
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
    fn yaml_front_matter_after_bom() {
        let text = "\u{feff}---\ndate: 2020-01-02\nsource: 'https://a.test/x'\n---\n";
        let page = run(text);
        assert_eq!(page.date, NaiveDate::from_ymd_opt(2020, 1, 2));
        assert_eq!(page.links.len(), 1);
        let link = &page.links[0];
        assert_eq!(
            link.ctx,
            Ctx::FrontMatter {
                quote: Quote::Single
            }
        );
        assert_eq!(&text[link.span.clone()], "https://a.test/x");
        assert_eq!(link.line, 3);
    }

    #[test]
    fn second_bom_is_not_front_matter() {
        let text = "\u{feff}\u{feff}---\na: 'https://a.test/x'\n---\n";
        let page = run(text);
        assert!(
            page.links
                .iter()
                .all(|l| !matches!(l.ctx, Ctx::FrontMatter { .. }))
        );
    }

    #[test]
    fn bom_without_front_matter_keeps_body_links() {
        let text = "\u{feff}see https://m.test/a\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        let link = &page.links[0];
        assert_eq!(link.ctx, Ctx::MdText);
        assert_eq!(&text[link.span.clone()], "https://m.test/a");
        assert_eq!(link.line, 1);
    }

    #[test]
    fn many_boms_do_not_overflow_the_stack() {
        let text = format!("{}---\n---\n", "\u{feff}".repeat(50_000));
        let _ = run(&text);
    }

    #[test]
    fn toml_front_matter_after_bom() {
        let text = "\u{feff}+++\ndate = 2020-01-02\nsource = \"https://a.test/x\"\n+++\n";
        let page = run(text);
        assert_eq!(page.date, NaiveDate::from_ymd_opt(2020, 1, 2));
        assert_eq!(page.links.len(), 1);
        let link = &page.links[0];
        assert_eq!(
            link.ctx,
            Ctx::FrontMatter {
                quote: Quote::Double
            }
        );
        assert_eq!(&text[link.span.clone()], "https://a.test/x");
        assert_eq!(link.line, 3);
    }

    #[test]
    fn toml_nested_array_of_tables_and_date() {
        let text = "+++\ntitle = \"T\"\ndate = 2024-03-05T10:00:00Z\n\n[[discussedOn]]\nurl = \"https://a.test/x\"\n\n[[discussedOn]]\nurl = 'https://b.test/y'\n+++\nbody\n";
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
    fn yaml_continued_plain_value_does_not_claim_a_later_link() {
        let text =
            "---\nnote: https://a.test/x\n  is worth a read\nsource: https://a.test/x\n---\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1, "{:?}", page.links);
        let l = &page.links[0];
        assert_eq!(l.url, "https://a.test/x");
        assert_eq!(l.line, 4);
        assert!(l.verbatim);
        assert_eq!(&text[l.span.clone()], "https://a.test/x");
        assert!(text[..l.span.start].ends_with("source: "));
    }

    #[test]
    fn yaml_comment_line_after_a_plain_value_keeps_its_token() {
        for comment in ["  # note", "  \t# note"] {
            let text = format!("---\nsource: https://a.test/x\n{comment}\n---\n");
            let page = run(&text);
            assert_eq!(page.links.len(), 1, "{comment:?}: {:?}", page.links);
            let l = &page.links[0];
            assert!(l.verbatim, "{comment:?}");
            assert_eq!(&text[l.span.clone()], "https://a.test/x");
            assert_eq!(l.line, 2);
        }
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
    fn yaml_plain_scalar_under_a_valueless_key_continues_against_the_key() {
        for head in ["a: &x", "a:", "a: !t"] {
            let text =
                format!("---\n{head}\n  https://a.test/p\n  more\nb: https://a.test/p\n---\n");
            let page = run(&text);
            assert_eq!(page.links.len(), 1, "{head}");
            let l = &page.links[0];
            assert_eq!((l.line, l.verbatim), (5, true), "{head}");
            assert_eq!(&text[l.span.clone()], "https://a.test/p", "{head}");
            assert_eq!(text[..l.span.start].matches('\n').count(), 4, "{head}");
        }
    }

    #[test]
    fn yaml_nested_structures_under_a_valueless_key_keep_their_links() {
        for body in [
            "a:\n  https://a.test/p\nb: x\n",
            "a:\n  - https://a.test/p\n  - more\n",
            "a:\n  url: https://a.test/p\n",
        ] {
            let text = format!("---\n{body}---\n");
            let page = run(&text);
            assert_eq!(page.links.len(), 1, "{body:?}");
            let l = &page.links[0];
            assert_eq!((l.line, l.verbatim), (3, true), "{body:?}");
            assert_eq!(&text[l.span.clone()], "https://a.test/p", "{body:?}");
        }
    }

    #[test]
    fn yaml_property_only_value_introduces_the_nested_block() {
        for text in [
            "---\nlinks: &l\n  - https://a.test/x\n---\n",
            "---\nmeta: !thing\n  url: https://a.test/x\n---\n",
            "---\nmeta: &m !thing # c\n  url: https://a.test/x\n---\n",
            "---\nlinks:\n  - &l\n    url: https://a.test/x\n---\n",
        ] {
            let page = run(text);
            assert_eq!(page.links.len(), 1, "{text:?}");
            assert_eq!(page.links[0].url, "https://a.test/x");
            assert!(page.links[0].verbatim, "{text:?}");
            assert_eq!(&text[page.links[0].span.clone()], "https://a.test/x");
        }
    }

    #[test]
    fn yaml_properties_before_a_value_are_not_part_of_it() {
        let fm = |quote| Ctx::FrontMatter { quote };
        let text = "---\na: &x https://a.test/z\n---\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        assert_eq!(page.links[0].ctx, fm(Quote::Bare));
        assert!(page.links[0].verbatim);
        assert_eq!(&text[page.links[0].span.clone()], "https://a.test/z");

        let text = "---\na: !!str 'https://a.test/q'\n---\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        assert_eq!(page.links[0].ctx, fm(Quote::Single));
        assert!(page.links[0].verbatim);
        assert_eq!(&text[page.links[0].span.clone()], "https://a.test/q");
    }

    #[test]
    fn yaml_tab_after_a_property_still_ends_it() {
        let text = "---\na: !foo\t|\n  https://a.test/b\nb: https://a.test/b\n---\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        assert!(page.links[0].verbatim);
        let span = page.links[0].span.clone();
        assert_eq!(&text[span.clone()], "https://a.test/b");
        assert_eq!(
            text[..span.start].matches('\n').count(),
            3,
            "link is on line 4"
        );
    }

    #[test]
    fn yaml_block_scalar_after_a_property_has_no_links() {
        let page = run("---\na: !foo |\n  https://a.test/b\n---\n");
        assert!(page.links.is_empty());
    }

    #[test]
    fn yaml_verbatim_tag_is_a_property() {
        let text = "---\na: !<tag:yaml.org,2002:str> https://a.test/v\n---\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        assert_eq!(page.links[0].ctx, Ctx::FrontMatter { quote: Quote::Bare });
        assert!(page.links[0].verbatim);
        assert_eq!(&text[page.links[0].span.clone()], "https://a.test/v");
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
        assert!(run("# title\nplain\n").links.is_empty());
        // Unclosed front matter is plain body text: only a bare URL, never a front matter value.
        let links = run("+++\nu = \"https://a.test\"\n").links;
        assert_eq!(links.len(), 1);
        assert_eq!(
            (links[0].url.as_str(), links[0].line, links[0].ctx),
            ("https://a.test", 2, Ctx::MdText)
        );
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

    fn body(text: &str) -> Vec<(String, u32, Ctx, bool)> {
        let page = run(text);
        assert_slices(text, &page);
        page.links
            .iter()
            .map(|l| (l.url.clone(), l.line, l.ctx, l.verbatim))
            .collect()
    }

    #[test]
    fn body_link_kinds_have_url_line_and_absolute_span() {
        let text = "---\ntitle: T\n---\nintro [a](https://a.test/x \"t https://a.test/x\") and\n![i](<https://b.test/img>)\n\nauto <https://c.test/z>\n\n<a href=\"https://d.test/h\">h</a> <img src='https://e.test/s'>\n\n<div>\n<a HREF = \"https://f.test/b\">b</a>\n</div>\n";
        let page = run(text);
        assert_slices(text, &page);
        let got: Vec<_> = page
            .links
            .iter()
            .map(|l| (l.url.as_str(), l.line, l.ctx, &text[l.span.clone()]))
            .collect();
        assert_eq!(
            got,
            [
                ("https://a.test/x", 4, Ctx::MdDest, "https://a.test/x"),
                ("https://b.test/img", 5, Ctx::MdDest, "https://b.test/img"),
                ("https://c.test/z", 7, Ctx::MdDest, "https://c.test/z"),
                ("https://d.test/h", 9, Ctx::Html, "https://d.test/h"),
                ("https://e.test/s", 9, Ctx::Html, "https://e.test/s"),
                ("https://f.test/b", 12, Ctx::Html, "https://f.test/b"),
            ]
        );
        // The span is file-absolute: the first one is the destination, not the title.
        assert_eq!(page.links[0].span.start, text.find("](").unwrap() + 2);
    }

    #[test]
    fn nested_image_in_link_gives_both_destinations() {
        let text = "[![i](https://a.test/i)](https://a.test/l)\n";
        let page = run(text);
        assert_slices(text, &page);
        let spans: Vec<_> = page.links.iter().map(|l| l.span.start).collect();
        assert_eq!(
            spans,
            [
                text.find("https://a.test/i").unwrap(),
                text.find("https://a.test/l").unwrap()
            ]
        );
    }

    #[test]
    fn code_yields_nothing() {
        let text = "```\n[a](https://a.test/1)\n<a href=\"https://a.test/2\">\n```\n\n    [b](https://a.test/3)\n    <https://a.test/4>\n\n~~~html\n<a href=\"https://a.test/5\">\n~~~\n\n`[c](https://a.test/6)` and ``<https://a.test/7>``\n";
        assert_eq!(body(text), []);
    }

    #[test]
    fn escaped_destination_is_not_verbatim() {
        let text = "pre\n\n[x](https://m.test/a\\_b)\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        let l = &page.links[0];
        assert_eq!(l.url, "https://m.test/a_b");
        assert!(!l.verbatim);
        assert_eq!(l.line, 3);
        assert_eq!(&text[l.span.clone()], "[x](https://m.test/a\\_b)");
    }

    #[test]
    fn reference_definitions_count_once_used_or_not() {
        let text = "see [one][a] and [one][a] and [a]\n\n[a]: https://a.test/used \"T\"\n[b]: <https://b.test/unused>\n";
        let got = body(text);
        assert_eq!(
            got,
            [
                ("https://a.test/used".to_owned(), 3, Ctx::MdDest, true),
                ("https://b.test/unused".to_owned(), 4, Ctx::MdDest, true),
            ]
        );
    }

    #[test]
    fn plain_inline_destination_with_title_stays_verbatim() {
        let text = "pre [x](https://m.test/a \"title\")\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        assert!(page.links[0].verbatim);
        assert_eq!(&text[page.links[0].span.clone()], "https://m.test/a");
    }

    #[test]
    fn escaped_destinations_fall_back_to_the_event_range() {
        for (text, url, whole) in [
            (
                "[x](https://m.test/a&amp;)\n",
                "https://m.test/a&",
                "[x](https://m.test/a&amp;)",
            ),
            (
                "[x](https://m.test/a\\\\)\n",
                "https://m.test/a\\",
                "[x](https://m.test/a\\\\)",
            ),
            (
                "[x](https://m.test/a\\_b \"https://m.test/a_b\")\n",
                "https://m.test/a_b",
                "[x](https://m.test/a\\_b \"https://m.test/a_b\")",
            ),
            (
                "[r]: https://m.test/a\\_b \"https://m.test/a_b\"\n",
                "https://m.test/a_b",
                "[r]: https://m.test/a\\_b \"https://m.test/a_b\"",
            ),
        ] {
            let page = run(text);
            assert_eq!(page.links.len(), 1, "{text}");
            let l = &page.links[0];
            assert_eq!(l.url, url, "{text}");
            assert!(!l.verbatim, "{text}");
            assert_eq!(text[l.span.clone()].trim_end(), whole, "{text}");
        }
    }

    #[test]
    fn html_attribute_entities_are_decoded_and_not_verbatim() {
        let text = "<a href=\"https://h.test/?a=1&amp;b=2&#x26;c=&#51;\">x</a>\n";
        let page = run(text);
        assert_eq!(page.links.len(), 1);
        let l = &page.links[0];
        assert_eq!(l.url, "https://h.test/?a=1&b=2&c=3");
        assert!(!l.verbatim);
        assert_eq!(
            &text[l.span.clone()],
            "https://h.test/?a=1&amp;b=2&#x26;c=&#51;"
        );
    }

    #[test]
    fn non_http_destinations_yield_nothing() {
        let text = "[a](mailto:someone) [b](rel/path) [c](#frag) [d](//h.test/x) <a href=\"/rel\">\n\n[e]: mailto:someone\n[f]: ../up\n";
        assert_eq!(body(text), []);
    }

    #[test]
    fn front_matter_is_not_parsed_as_markdown() {
        let text = "---\nu: https://a.test/x\nmd: \"[a](https://b.test/y)\"\n---\nbody\n";
        let urls: Vec<_> = run(text).links.into_iter().map(|l| l.url).collect();
        assert_eq!(urls, ["https://a.test/x"]);
    }

    #[test]
    fn bare_urls_are_trimmed_and_balanced() {
        let text = "see https://m.test/a_b_(c)&x=1 now\n\n(see https://m.test/x).\n\n{{< figure src=\"https://m.test/s\" alt=\"x\" >}}\n\n- tight https://m.test/t, ok\n";
        let page = run(text);
        assert_slices(text, &page);
        let got: Vec<_> = page
            .links
            .iter()
            .map(|l| (l.url.as_str(), l.line, l.ctx, l.verbatim))
            .collect();
        assert_eq!(
            got,
            [
                ("https://m.test/a_b_(c)&x=1", 1, Ctx::MdText, true),
                ("https://m.test/x", 3, Ctx::MdText, true),
                ("https://m.test/s", 5, Ctx::MdText, true),
                ("https://m.test/t", 7, Ctx::MdText, true),
            ]
        );
    }

    #[test]
    fn bare_url_in_file_without_front_matter() {
        let page = run("# title\n\nsee https://a.test/x here\n");
        let got: Vec<_> = page
            .links
            .iter()
            .map(|l| (l.url.as_str(), l.line))
            .collect();
        assert_eq!(got, [("https://a.test/x", 3)]);
    }

    #[test]
    fn tight_list_urls_survive_inline_markup() {
        let cases = [
            ("- https://m.test/a\\_b\n", "https://m.test/a_b", false),
            (
                "- see https://m.test/**b**/c\n",
                "https://m.test/**b**/c",
                true,
            ),
            ("> - https://m.test/*q*\n", "https://m.test/*q", true),
        ];
        for (text, url, verbatim) in cases {
            let page = run(text);
            assert_slices(text, &page);
            let got: Vec<_> = page
                .links
                .iter()
                .map(|l| (l.url.as_str(), l.verbatim))
                .collect();
            assert_eq!(got, [(url, verbatim)], "{text:?}");
        }
    }

    #[test]
    fn bare_url_trimming_keeps_escapes_whole() {
        let text = "see https://m.test/a\\_ here\n\nhttps://m.test/b_\n\nhttps://m.test/c\\\\_\n";
        let page = run(text);
        let spans: Vec<(&str, &str)> = page
            .links
            .iter()
            .map(|l| (l.url.as_str(), &text[l.span.clone()]))
            .collect();
        assert_eq!(
            spans,
            [
                ("https://m.test/a", "https://m.test/a"),
                ("https://m.test/b", "https://m.test/b"),
                ("https://m.test/c\\", "https://m.test/c\\\\"),
            ]
        );
        let hostile = format!("https://m.test/a{}\n", "\\_".repeat(5000));
        let page = run(&hostile);
        assert_eq!(page.links[0].url, "https://m.test/a");
    }

    #[test]
    fn bare_urls_drop_markdown_delimiters() {
        let cases = [
            ("**see https://m.test/bold**\n", "https://m.test/bold"),
            ("[see https://m.test/br]\n", "https://m.test/br"),
            ("_https://m.test/ital_\n", "https://m.test/ital"),
        ];
        for (text, url) in cases {
            let got: Vec<_> = run(text).links.into_iter().map(|l| l.url).collect();
            assert_eq!(got, [url], "{text:?}");
        }
    }

    #[test]
    fn bare_urls_decode_escapes_and_entities() {
        let text = "see https://m.test/a\\_b\n\nhttps://m.test/a?b=c&amp;d=1\n";
        let page = run(text);
        let got: Vec<_> = page
            .links
            .iter()
            .map(|l| (l.url.as_str(), l.verbatim))
            .collect();
        assert_eq!(
            got,
            [
                ("https://m.test/a_b", false),
                ("https://m.test/a?b=c&d=1", false)
            ]
        );
        assert_eq!(&text[page.links[0].span.clone()], "https://m.test/a\\_b");
    }

    #[test]
    fn bare_urls_skip_links_code_and_html() {
        let text = "[x](https://m.test/a) and <https://m.test/b> and `https://m.test/c` and <a href=\"https://m.test/d\">d</a> and [https://m.test/e](https://m.test/f)\n\n[r]: https://m.test/g\n";
        let urls: Vec<_> = body(text).into_iter().map(|l| (l.0, l.2)).collect();
        assert_eq!(
            urls,
            [
                ("https://m.test/a".to_owned(), Ctx::MdDest),
                ("https://m.test/b".to_owned(), Ctx::MdDest),
                ("https://m.test/d".to_owned(), Ctx::Html),
                ("https://m.test/f".to_owned(), Ctx::MdDest),
                ("https://m.test/g".to_owned(), Ctx::MdDest),
            ]
        );
    }
}
