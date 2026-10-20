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

    if format == Format::Yaml {
        yaml_links(page, lines, block.clone(), block_text, &values.urls);
        return;
    }

    let all_urls = values.urls;
    let mut remaining = all_urls.clone();
    let tokens = lex_toml(block_text);
    // The lexer can emit a phantom token whose text equals a real value elsewhere, and
    // pairing by text would let it take the verbatim span. A link is verbatim only when
    // the parser reads the token's span as exactly one of the values.
    let verify = Verify {
        block_text,
        date_key: &cfg.date_key,
        urls: &all_urls,
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
                verify.is_verbatim(content, &token.content),
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
        if token.quote != Quote::Double {
            continue;
        }
        let Some(decoded) = decode_toml(&block_text[token.content.clone()]) else {
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

/// Records the URL values of a YAML block. They are checked but never applied, so each span
/// only has to point near the value: the first occurrence of its text after the previous
/// one, or the whole block when the text does not occur there (an escaped value).
fn yaml_links(
    page: &mut Page,
    lines: &Lines,
    block: Range<usize>,
    block_text: &str,
    urls: &[String],
) {
    let mut cursor = 0;
    for url in urls {
        let span = match block_text[cursor..].find(url.as_str()) {
            Some(i) => {
                let start = block.start + cursor + i;
                cursor += i + url.len();
                start..start + url.len()
            }
            None => block.clone(),
        };
        page.links.push(link(
            lines,
            url,
            span,
            Ctx::FrontMatter {
                quote: Quote::Double,
            },
            false,
        ));
    }
}

/// What the parser read from a front-matter block, to confirm a token's span is a value.
struct Verify<'a> {
    block_text: &'a str,
    date_key: &'a str,
    urls: &'a [String],
}

impl Verify<'_> {
    /// Whether replacing `span` with a sentinel URL makes the parser read the sentinel
    /// exactly once and one copy fewer of `url`. A span inside a larger scalar, or a value
    /// that is also read through an alias, fails.
    fn is_verbatim(&self, url: &str, span: &Range<usize>) -> bool {
        let sentinel = (0..)
            .map(|n| format!("https://rotguard-sentinel.invalid/{n}"))
            .find(|s| !self.block_text.contains(s.as_str()))
            .expect("an unbounded range has an unused sentinel");
        let mut probe = String::with_capacity(self.block_text.len() + sentinel.len());
        probe.push_str(&self.block_text[..span.start]);
        probe.push_str(&sentinel);
        probe.push_str(&self.block_text[span.end..]);
        let Some(values) = parse_values(Format::Toml, &probe, self.date_key) else {
            return false;
        };
        let count = |urls: &[String], target: &str| urls.iter().filter(|u| *u == target).count();
        count(&values.urls, &sentinel) == 1 && count(&values.urls, url) + 1 == count(self.urls, url)
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
                    let decoded = decode_entities(value.as_str());
                    if keep(&decoded) {
                        let span = range.start + value.start()..range.start + value.end();
                        // Raw-HTML links are never written: manual fix.
                        page.links
                            .push(link(lines, &decoded, span, Ctx::Html, false));
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
                // Bare URLs are never written: manual fix.
                false,
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

/// Decodes the content of a TOML basic string token the way the parser would.
fn decode_toml(content: &str) -> Option<String> {
    let table: toml::Table = toml::from_str(&format!("v = \"{content}\"")).ok()?;
    table.get("v")?.as_str().map(str::to_owned)
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
/// Returns the index of the closing quote.
fn closing_quote(s: &str, from: usize, quote: u8, backslash: bool) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => return None,
            b'\\' if backslash => i += 1,
            b if b == quote => return Some(i),
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
                let Some(end) = closing_quote(block, i + 1, q, q == b'"') else {
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
    fn many_boms_do_not_overflow_the_stack() {
        let text = format!("{}---\n---\n", "\u{feff}".repeat(50_000));
        let _ = run(&text);
    }

    #[test]
    fn toml_front_matter_links_dates_and_spans() {
        use Quote::{Double as D, Single as S};
        // (text, date, expected (url, line, quote, raw span when the link is not verbatim))
        type Row = (
            &'static str,
            Option<NaiveDate>,
            &'static [(&'static str, u32, Quote, Option<&'static str>)],
        );
        let rows: &[Row] = &[
            (
                "+++\na = \"https://a.test/p\"\nb = \"https://a.test/p\"\n+++\n",
                None,
                &[
                    ("https://a.test/p", 2, D, None),
                    ("https://a.test/p", 3, D, None),
                ],
            ),
            (
                "\u{feff}+++\ndate = 2020-01-02\nsource = \"https://a.test/x\"\n+++\n",
                NaiveDate::from_ymd_opt(2020, 1, 2),
                &[("https://a.test/x", 3, D, None)],
            ),
            (
                "+++\ntitle = \"T\"\ndate = 2024-03-05T10:00:00Z\n\n[[discussedOn]]\nurl = \"https://a.test/x\"\n\n[[discussedOn]]\nurl = 'https://b.test/y'\n+++\nbody\n",
                NaiveDate::from_ymd_opt(2024, 3, 5),
                &[
                    ("https://a.test/x", 6, D, None),
                    ("https://b.test/y", 9, S, None),
                ],
            ),
            (
                "+++\ndate = \"2023-12-31T08:00:00+01:00\"\n+++\n",
                NaiveDate::from_ymd_opt(2023, 12, 31),
                &[],
            ),
            (
                "+++\n[\"https://a.test/x\"]\nnote = \"https://a.test/x\"\n+++\n",
                None,
                &[("https://a.test/x", 3, D, None)],
            ),
            (
                "+++\n[[\"https://a.test/x\"]]\nnote = \"https://a.test/x\"\n+++\n",
                None,
                &[("https://a.test/x", 3, D, None)],
            ),
            (
                "+++\nl = [\n  [\"https://a.test/x\"],\n  'https://b.test/y',\n]\n+++\n",
                None,
                &[
                    ("https://a.test/x", 3, D, None),
                    ("https://b.test/y", 4, S, None),
                ],
            ),
            (
                "+++\ntitle = \"T\"\nu = \"https://e.test/a\\u0062\"\n+++\n",
                None,
                &[("https://e.test/ab", 3, D, Some("https://e.test/a\\u0062"))],
            ),
        ];
        for (text, date, want) in rows {
            let page = run(text);
            assert_eq!(page.date, *date, "{text:?}");
            let got: Vec<_> = page
                .links
                .iter()
                .map(|l| {
                    let Ctx::FrontMatter { quote } = l.ctx else {
                        panic!("{text:?}: {:?}", l.ctx)
                    };
                    let raw = (!l.verbatim).then(|| &text[l.span.clone()]);
                    (l.url.as_str(), l.line, quote, raw)
                })
                .collect();
            assert_eq!(got, *want, "{text:?}");
            assert_slices(text, &page);
        }
    }

    #[test]
    fn toml_url_inside_another_string_is_never_verbatim() {
        // The lexer sees a token inside `a`'s multi-line string; only the sentinel re-parse
        // stops `--apply` from writing there instead of at `b`.
        let text = "+++\na = \"\"\"x \\\"\"\" \"https://a.test/p\" y\"\"\"\nb = \"https://a.test/p\"\n+++\n";
        let page = run(text);
        let b = text.find("b = ").unwrap();
        assert!(
            page.links.iter().any(|l| l.url == "https://a.test/p"),
            "{text:?}"
        );
        for l in page.links.iter().filter(|l| l.verbatim) {
            assert!(l.span.start > b, "verbatim span {:?} is inside `a`", l.span);
        }
        assert_slices(text, &page);
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
    fn yaml_values_are_reported_never_applied() {
        // (name, text, expected (url, line, span is the url's text), expected date)
        type Row = (
            &'static str,
            &'static str,
            &'static [(&'static str, u32, bool)],
            Option<NaiveDate>,
        );
        let rows: &[Row] = &[
            (
                "nested mapping, list and date",
                "---\ndate: 2024-03-05\nnested:\n  deep:\n    link: https://a.test/x  # c\nl:\n  - \"https://b.test/y\"\n  - 'https://c.test/z'\n---\n",
                &[
                    ("https://a.test/x", 5, true),
                    ("https://b.test/y", 7, true),
                    ("https://c.test/z", 8, true),
                ],
                NaiveDate::from_ymd_opt(2024, 3, 5),
            ),
            (
                "flow list",
                "---\ntags: [https://a.test/p]\n---\n",
                &[("https://a.test/p", 2, true)],
                None,
            ),
            (
                "repeated url on two lines",
                "---\na: https://a.test/p\nb: https://a.test/p\n---\n",
                &[("https://a.test/p", 2, true), ("https://a.test/p", 3, true)],
                None,
            ),
            (
                "escaped value gets the whole block",
                "---\ntitle: T\nu: \"https://e.test/a\\x62\"\nv: https://a.test/z\n---\n",
                &[
                    ("https://e.test/ab", 2, false),
                    ("https://a.test/z", 4, true),
                ],
                None,
            ),
            (
                "tagged values",
                "---\nmeta: !thing\n  url: https://a.test/x\nv: !<tag:a.test,2024:u> https://a.test/y\n---\n",
                &[("https://a.test/x", 3, true), ("https://a.test/y", 4, true)],
                None,
            ),
            (
                "byte-order mark and date string",
                "\u{feff}---\ndate: \"2022-01-02 10:00\"\nsource: 'https://a.test/x'\n---\n",
                &[("https://a.test/x", 3, true)],
                NaiveDate::from_ymd_opt(2022, 1, 2),
            ),
        ];
        for (name, text, expected, date) in rows {
            let page = run(text);
            assert_eq!(page.date, *date, "{name}");
            let got: Vec<_> = page
                .links
                .iter()
                .map(|l| (l.url.as_str(), l.line, text[l.span.clone()] == l.url))
                .collect();
            assert_eq!(got, *expected, "{name}");
            for l in &page.links {
                assert!(!l.verbatim, "{name}");
                assert_eq!(
                    l.ctx,
                    Ctx::FrontMatter {
                        quote: Quote::Double
                    },
                    "{name}"
                );
            }
        }
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
    fn non_link_inputs_yield_nothing() {
        for text in [
            "# title\nplain\n",
            "+++\nu = \n+++\n",
            "+++\na = \"mailto:someone\"\nb = \"/rel\"\nc = \"//h.test/x\"\nd = \"https://\"\n+++\n",
            "```\n[a](https://a.test/1)\n<a href=\"https://a.test/2\">\n```\n\n    [b](https://a.test/3)\n    <https://a.test/4>\n\n~~~html\n<a href=\"https://a.test/5\">\n~~~\n\n`[c](https://a.test/6)` and ``<https://a.test/7>``\n",
            "[a](mailto:someone) [b](rel/path) [c](#frag) [d](//h.test/x) <a href=\"/rel\">\n\n[e]: mailto:someone\n[f]: ../up\n",
        ] {
            assert_eq!(body(text), [], "{text:?}");
        }
    }

    #[test]
    fn bare_urls_are_trimmed_decoded_and_never_verbatim() {
        // (text, expected (url, line, span text))
        type Row = (&'static str, &'static [(&'static str, u32, &'static str)]);
        let rows: &[Row] = &[
            (
                "see https://m.test/a_b_(c)&x=1 now\n\n(see https://m.test/x).\n\n{{< figure src=\"https://m.test/s\" alt=\"x\" >}}\n\n- tight https://m.test/t, ok\n",
                &[
                    (
                        "https://m.test/a_b_(c)&x=1",
                        1,
                        "https://m.test/a_b_(c)&x=1",
                    ),
                    ("https://m.test/x", 3, "https://m.test/x"),
                    ("https://m.test/s", 5, "https://m.test/s"),
                    ("https://m.test/t", 7, "https://m.test/t"),
                ],
            ),
            (
                "# title\n\nsee https://a.test/x here\n",
                &[("https://a.test/x", 3, "https://a.test/x")],
            ),
            (
                "+++\nu = \"https://a.test\"\n",
                &[("https://a.test", 2, "https://a.test")],
            ),
            (
                "\u{feff}see https://m.test/a\n",
                &[("https://m.test/a", 1, "https://m.test/a")],
            ),
            (
                "- https://m.test/a\\_b\n",
                &[("https://m.test/a_b", 1, "https://m.test/a\\_b")],
            ),
            (
                "- see https://m.test/**b**/c\n",
                &[("https://m.test/**b**/c", 1, "https://m.test/**b**/c")],
            ),
            (
                "> - https://m.test/*q*\n",
                &[("https://m.test/*q", 1, "https://m.test/*q")],
            ),
            (
                "see https://m.test/a\\_ here\n\nhttps://m.test/b_\n\nhttps://m.test/c\\\\_\n",
                &[
                    ("https://m.test/a", 1, "https://m.test/a"),
                    ("https://m.test/b", 3, "https://m.test/b"),
                    ("https://m.test/c\\", 5, "https://m.test/c\\\\"),
                ],
            ),
            (
                "**see https://m.test/bold**\n",
                &[("https://m.test/bold", 1, "https://m.test/bold")],
            ),
            (
                "[see https://m.test/br]\n",
                &[("https://m.test/br", 1, "https://m.test/br")],
            ),
            (
                "_https://m.test/ital_\n",
                &[("https://m.test/ital", 1, "https://m.test/ital")],
            ),
            (
                "see https://m.test/a\\_b\n\nhttps://m.test/a?b=c&amp;d=1\n",
                &[
                    ("https://m.test/a_b", 1, "https://m.test/a\\_b"),
                    (
                        "https://m.test/a?b=c&d=1",
                        3,
                        "https://m.test/a?b=c&amp;d=1",
                    ),
                ],
            ),
        ];
        for (text, want) in rows {
            let page = run(text);
            let got: Vec<_> = page
                .links
                .iter()
                .map(|l| (l.url.as_str(), l.line, &text[l.span.clone()]))
                .collect();
            assert_eq!(got, *want, "{text:?}");
            assert!(
                page.links
                    .iter()
                    .all(|l| l.ctx == Ctx::MdText && !l.verbatim),
                "{text:?}"
            );
        }
        let hostile = format!("https://m.test/a{}\n", "\\_".repeat(5000));
        assert_eq!(run(&hostile).links[0].url, "https://m.test/a");
    }

    #[test]
    fn second_bom_is_not_front_matter() {
        let page = run("\u{feff}\u{feff}---\na: 'https://a.test/x'\n---\n");
        assert!(
            page.links
                .iter()
                .all(|l| !matches!(l.ctx, Ctx::FrontMatter { .. }))
        );
    }

    #[test]
    fn front_matter_is_not_parsed_as_markdown() {
        let text = "---\nu: https://a.test/x\nmd: \"[a](https://b.test/y)\"\n---\nbody\n";
        let urls: Vec<_> = run(text).links.into_iter().map(|l| l.url).collect();
        assert_eq!(urls, ["https://a.test/x"]);
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
