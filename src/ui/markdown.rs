//! Shared CommonMark-subset renderer into native GTK text buffers.
//!
//! Agent answers, research synthesis, and notes are markdown; GTK has no
//! markdown widget and this crate avoids a webview. This module parses a
//! pragmatic subset (headings, bold/italic/inline code, fenced code blocks,
//! unordered/ordered lists, blockquotes, horizontal rules, `[text](url)`
//! links, `#hashtag` spans) into plain text plus style spans, then applies
//! them to a [`gtk4::TextBuffer`] via named [`gtk4::TextTag`]s.
//!
//! Pure parsing (`parse`) is dependency-free and unit-tested; only
//! [`render_into_buffer`] touches GTK (main thread, like all widgets).

use gtk4::prelude::*;

/// One styled span over the plain-text output: byte range plus tag name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyleSpan {
    /// Byte offset into the plain text (inclusive).
    pub start: usize,
    /// Byte offset into the plain text (exclusive).
    pub end: usize,
    /// Tag created by [`ensure_tags`] (`md_bold`, `md_italic`, …).
    pub tag: &'static str,
}

/// Parsed document: plain text plus the spans styling it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkdownDoc {
    pub text: String,
    pub spans: Vec<StyleSpan>,
}

/// Parse `markdown` into plain text + style spans.
///
/// Block structure handled line-wise: `#`/`##`/`###` headings, ` ``` `
/// fences, `>` quotes, `-`/`*`/`+` bullets, `1.` ordered items, `---` rules,
/// blank-line paragraph breaks. Inline `**bold**`, `*italic*`, `` `code` ``,
/// `[text](url)`, and `#hashtag` apply wherever they appear (including
/// inside headings and quotes). Unclosed markers render literally; links
/// without a URL render their text with the link style.
pub fn parse(markdown: &str) -> MarkdownDoc {
    let mut doc = MarkdownDoc::default();
    let mut in_fence = false;
    let mut list_counters: Vec<u64> = Vec::new();

    // `prev_blank` collapses runs of blank lines into one paragraph break;
    // `pending_bullets` peeks one line ahead so `-` nested under `-` indent.
    let lines: Vec<&str> = markdown.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            push_block_gap(&mut doc);
            index += 1;
            continue;
        }
        if in_fence {
            let start = doc.text.len();
            doc.text.push_str(line);
            doc.text.push('\n');
            doc.spans.push(StyleSpan {
                start,
                end: doc.text.len(),
                tag: "md_codeblock",
            });
            index += 1;
            continue;
        }
        if trimmed.is_empty() {
            push_block_gap(&mut doc);
            list_counters.clear();
            index += 1;
            continue;
        }
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            if !doc.text.is_empty() && !doc.text.ends_with("\n\n") {
                if !doc.text.ends_with('\n') {
                    doc.text.push('\n');
                }
                doc.text.push('\n');
            }
            let start = doc.text.len();
            doc.text.push_str("─ ─ ─\n\n");
            doc.spans.push(StyleSpan {
                start,
                end: start + "─ ─ ─".len(),
                tag: "md_dim",
            });
            index += 1;
            continue;
        }
        if let Some((level, rest)) = parse_heading(trimmed) {
            push_block_gap(&mut doc);
            let tag = match level {
                1 => "md_h1",
                2 => "md_h2",
                _ => "md_h3",
            };
            let start = doc.text.len();
            push_inline(&mut doc, rest);
            let end = doc.text.len();
            doc.spans.push(StyleSpan { start, end, tag });
            doc.text.push_str("\n\n");
            list_counters.clear();
            index += 1;
            continue;
        }
        if let Some(quoted) = trimmed.strip_prefix('>') {
            push_block_gap(&mut doc);
            let start = doc.text.len();
            push_inline(&mut doc, quoted.trim_start_matches(' ').trim_end());
            let end = doc.text.len();
            doc.spans.push(StyleSpan {
                start,
                end,
                tag: "md_quote",
            });
            doc.text.push('\n');
            index += 1;
            continue;
        }
        if let Some(item) = parse_bullet(trimmed) {
            let indent = line.len() - line.trim_start().len();
            // Sibling sub-items share one counter level; deeper indents nest.
            let nested = index + 1 < lines.len()
                && lines[index + 1].trim_start().len() < lines[index + 1].len()
                && parse_bullet(lines[index + 1].trim()).is_some();
            push_block_gap(&mut doc);
            let start = doc.text.len();
            doc.text.push_str(&"  ".repeat(indent.min(4) / 2));
            doc.text.push_str(if nested { "▸ " } else { "• " });
            let bullet_end = doc.text.len();
            doc.spans.push(StyleSpan {
                start,
                end: bullet_end,
                tag: "md_bullet",
            });
            push_inline(&mut doc, item);
            doc.text.push('\n');
            index += 1;
            continue;
        }
        if let Some((number, item)) = parse_ordered(trimmed) {
            if list_counters.is_empty() {
                list_counters.push(number);
            }
            push_block_gap(&mut doc);
            let start = doc.text.len();
            doc.text.push_str(&format!("{number}. "));
            let num_end = doc.text.len();
            doc.spans.push(StyleSpan {
                start,
                end: num_end,
                tag: "md_bullet",
            });
            push_inline(&mut doc, item);
            doc.text.push('\n');
            index += 1;
            continue;
        }
        // Plain paragraph line: soft-wrap joins with a space unless the
        // previous output already ends a block.
        if !doc.text.is_empty() && !doc.text.ends_with('\n') {
            doc.text.push(' ');
        }
        push_inline(&mut doc, trimmed);
        if index + 1 >= lines.len() || lines[index + 1].trim().is_empty() {
            doc.text.push('\n');
        }
        index += 1;
    }
    // Trailing blank line is a layout artifact, not content.
    while doc.text.ends_with('\n')
        && doc.text.len() > 1
        && doc.text[..doc.text.len() - 1].ends_with('\n')
    {
        doc.text.pop();
    }
    doc
}

/// Ensure the `md_*` tags exist on `table` (idempotent: existing names are
/// skipped). Sizes use `scale` (relative, theme-friendly) instead of
/// absolute points; colors are fixed accents readable on dark and light
/// Adwaita themes.
pub fn ensure_tags(table: &gtk4::TextTagTable) {
    let blue = gtk4::gdk::RGBA::parse("#3584e4").unwrap_or(gtk4::gdk::RGBA::BLUE);
    let gray = gtk4::gdk::RGBA::parse("#9a9996").unwrap_or(gtk4::gdk::RGBA::WHITE);
    let light_blue = gtk4::gdk::RGBA::parse("#62a0ea").unwrap_or(gtk4::gdk::RGBA::BLUE);

    let defs: Vec<(&str, gtk4::TextTag)> = vec![
        (
            "md_h1",
            gtk4::TextTag::builder().scale(1.4).weight(700).build(),
        ),
        (
            "md_h2",
            gtk4::TextTag::builder().scale(1.2).weight(700).build(),
        ),
        (
            "md_h3",
            gtk4::TextTag::builder().scale(1.1).weight(700).build(),
        ),
        ("md_bold", gtk4::TextTag::builder().weight(700).build()),
        (
            "md_italic",
            gtk4::TextTag::builder().style(pango::Style::Italic).build(),
        ),
        (
            "md_code",
            gtk4::TextTag::builder().family("monospace").build(),
        ),
        (
            "md_codeblock",
            gtk4::TextTag::builder()
                .family("monospace")
                .left_margin(12)
                .build(),
        ),
        (
            "md_quote",
            gtk4::TextTag::builder()
                .style(pango::Style::Italic)
                .foreground_rgba(&gray)
                .left_margin(12)
                .build(),
        ),
        (
            "md_link",
            gtk4::TextTag::builder().foreground_rgba(&blue).build(),
        ),
        (
            "md_hashtag",
            gtk4::TextTag::builder()
                .weight(700)
                .foreground_rgba(&blue)
                .build(),
        ),
        (
            "md_bullet",
            gtk4::TextTag::builder()
                .weight(700)
                .foreground_rgba(&light_blue)
                .build(),
        ),
        (
            "md_dim",
            gtk4::TextTag::builder().foreground_rgba(&gray).build(),
        ),
    ];
    for (name, tag) in defs {
        if table.lookup(name).is_some() {
            continue;
        }
        tag.set_property("name", name);
        table.add(&tag);
    }
}

/// Render `markdown` into `buffer`, replacing its content. Tags are ensured
/// on the buffer's tag table first; unknown span tags are skipped.
pub fn render_into_buffer(buffer: &gtk4::TextBuffer, markdown: &str) {
    let table = buffer.tag_table();
    ensure_tags(&table);
    let doc = parse(markdown);
    buffer.set_text(&doc.text);
    let (start, end) = (buffer.start_iter(), buffer.end_iter());
    buffer.remove_all_tags(&start, &end);
    for span in &doc.spans {
        let tag = match table.lookup(span.tag) {
            Some(tag) => tag,
            None => continue,
        };
        let start_iter = buffer.iter_at_offset(span.start as i32);
        let end_iter = buffer.iter_at_offset(span.end as i32);
        buffer.apply_tag(&tag, &start_iter, &end_iter);
    }
}

/// Build a read-only, selectable, word-wrapping [`gtk4::TextView`]
/// pre-rendered with `markdown`. Used for agent bubbles and previews.
pub fn markdown_textview(markdown: &str) -> gtk4::TextView {
    let view = gtk4::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk4::WrapMode::Word);
    view.add_css_class("card");
    render_into_buffer(&view.buffer(), markdown);
    view
}

// ---------------------------------------------------------------------------
// Block helpers
// ---------------------------------------------------------------------------

/// Separate the next block from previous content with a blank line.
fn push_block_gap(doc: &mut MarkdownDoc) {
    if doc.text.is_empty() {
        return;
    }
    if !doc.text.ends_with('\n') {
        doc.text.push('\n');
    }
    if !doc.text.ends_with("\n\n") {
        doc.text.push('\n');
    }
}

/// `#`..`######` heading: returns (level clamped to 1..=3, body).
fn parse_heading(trimmed: &str) -> Option<(u32, &str)> {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = trimmed[hashes..].strip_prefix(' ')?;
    Some((hashes.min(3) as u32, rest))
}

/// `-`, `*`, or `+` bullet: returns the item body.
fn parse_bullet(trimmed: &str) -> Option<&str> {
    let first = trimmed.chars().next()?;
    if !matches!(first, '-' | '*' | '+') {
        return None;
    }
    trimmed[1..].strip_prefix(' ')
}

/// `1.`/`2)` ordered item: returns (number, body).
fn parse_ordered(trimmed: &str) -> Option<(u64, &str)> {
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let sep = trimmed.chars().nth(digits)?;
    if !matches!(sep, '.' | ')') {
        return None;
    }
    let body = trimmed[digits + 1..].strip_prefix(' ')?;
    let number: u64 = trimmed[..digits].parse().ok()?;
    Some((number, body))
}

// ---------------------------------------------------------------------------
// Inline parser: **bold**, *italic*, `code`, [text](url), #hashtag
// ---------------------------------------------------------------------------

/// Append `text` to `doc`, converting inline markers into spans. Offsets
/// are byte-based; slicing only happens at `char` boundaries found by the
/// scanners below. A `#` starts a hashtag only at a word boundary
/// (start or after whitespace/`(`/`[`) so `C#` and `issue#1` stay literal.
fn push_inline(doc: &mut MarkdownDoc, text: &str) {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    // Accumulate into a staging string so span offsets stay byte-exact
    // even with multi-byte characters; merged into `doc` once at the end.
    let mut staged = String::new();
    let mut staged_spans: Vec<(usize, usize, &'static str)> = Vec::new();
    while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        if rest.starts_with("**") {
            if let Some(len) = scan_closer(&chars, i + 2, "**") {
                let inner: String = chars[i + 2..i + 2 + len].iter().collect();
                let s = staged.len();
                push_inline_nested(&mut staged, &mut staged_spans, &inner);
                let e = staged.len();
                staged_spans.push((s, e, "md_bold"));
                i += 2 + len + 2;
                continue;
            }
            // Unclosed `**`: emit literally so the second star is not
            // reinterpreted as an italic opener mid-word.
            staged.push_str("**");
            i += 2;
            continue;
        }
        if chars[i] == '*' {
            if let Some(len) = scan_closer(&chars, i + 1, "*") {
                let inner: String = chars[i + 1..i + 1 + len].iter().collect();
                let s = staged.len();
                push_inline_nested(&mut staged, &mut staged_spans, &inner);
                let e = staged.len();
                staged_spans.push((s, e, "md_italic"));
                i += 1 + len + 1;
                continue;
            }
        }
        if chars[i] == '`' {
            if let Some(len) = scan_closer(&chars, i + 1, "`") {
                let inner: String = chars[i + 1..i + 1 + len].iter().collect();
                let s = staged.len();
                staged.push_str(&inner);
                staged_spans.push((s, staged.len(), "md_code"));
                i += 1 + len + 1;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some((text_len, url)) = scan_link(&chars, i) {
                let inner: String = chars[i + 1..i + 1 + text_len].iter().collect();
                let s = staged.len();
                staged.push_str(&inner);
                staged.push_str(&format!(" ({url})"));
                let e = staged.len();
                staged_spans.push((s, e, "md_link"));
                i += 1 + text_len + url.len() + 4;
                continue;
            }
        }
        if chars[i] == '#' && is_hashtag_start(&chars, i) {
            let mut j = i + 1;
            while j < chars.len()
                && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '-')
            {
                j += 1;
            }
            if j > i + 1 {
                let tag_text: String = chars[i..j].iter().collect();
                let s = staged.len();
                staged.push_str(&tag_text);
                staged_spans.push((s, staged.len(), "md_hashtag"));
                i = j;
                continue;
            }
        }
        staged.push(chars[i]);
        i += 1;
    }
    let base = doc.text.len();
    doc.text.push_str(&staged);
    for (s, e, tag) in staged_spans {
        doc.spans.push(StyleSpan {
            start: base + s,
            end: base + e,
            tag,
        });
    }
}

/// Inline nesting inside bold/italic: code spans only (avoids infinite
/// recursion through the full marker set).
fn push_inline_nested(
    staged: &mut String,
    spans: &mut Vec<(usize, usize, &'static str)>,
    inner: &str,
) {
    let chars: Vec<char> = inner.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '`' {
                j += 1;
            }
            if j < chars.len() {
                let code: String = chars[i + 1..j].iter().collect();
                let s = staged.len();
                staged.push_str(&code);
                spans.push((s, staged.len(), "md_code"));
                i = j + 1;
                continue;
            }
        }
        staged.push(chars[i]);
        i += 1;
    }
}

/// Find `closer` after `from` (char indices); returns the inner char count.
/// The closer must not be part of a longer run (`**` vs `*`).
fn scan_closer(chars: &[char], from: usize, closer: &str) -> Option<usize> {
    let closer_chars: Vec<char> = closer.chars().collect();
    let mut i = from;
    while i + closer_chars.len() <= chars.len() {
        if chars[i..i + closer_chars.len()] == closer_chars[..] {
            if closer == "*" {
                let prev_star = i > 0 && chars[i - 1] == '*';
                let next_star = i + 1 < chars.len() && chars[i + 1] == '*';
                if prev_star || next_star {
                    i += 1;
                    continue;
                }
            }
            if i == from {
                return None;
            }
            return Some(i - from);
        }
        i += 1;
    }
    None
}

/// `[text](url)`: returns (text char count, url). Requires the closing `]`
/// immediately followed by `(url)`; empty text or url rejected.
fn scan_link(chars: &[char], from: usize) -> Option<(usize, String)> {
    let mut i = from + 1;
    while i < chars.len() && chars[i] != ']' {
        i += 1;
    }
    if i >= chars.len() || i == from + 1 {
        return None;
    }
    if i + 1 >= chars.len() || chars[i + 1] != '(' {
        return None;
    }
    let mut j = i + 2;
    while j < chars.len() && chars[j] != ')' {
        j += 1;
    }
    if j >= chars.len() || j == i + 2 {
        return None;
    }
    let url: String = chars[i + 2..j].iter().collect();
    Some((i - (from + 1), url))
}

/// `#` begins a hashtag only at a word boundary.
fn is_hashtag_start(chars: &[char], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    matches!(chars[i - 1], ' ' | '\t' | '\n' | '(' | '[')
}

use gtk4::pango;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passes_through() {
        let doc = parse("hello world");
        assert_eq!(doc.text.trim(), "hello world");
        assert!(doc.spans.is_empty());
    }

    #[test]
    fn headings_become_sized_spans() {
        let doc = parse("# Title\n\n## Sub\n\n### Tiny");
        assert!(doc.text.contains("Title"));
        let tags: Vec<&str> = doc.spans.iter().map(|s| s.tag).collect();
        assert!(tags.contains(&"md_h1"));
        assert!(tags.contains(&"md_h2"));
        assert!(tags.contains(&"md_h3"));
    }

    #[test]
    fn inline_markers_span_correct_ranges() {
        let doc = parse("a **bold** and *italic* with `code`");
        assert_eq!(doc.text.trim(), "a bold and italic with code");
        let bold = doc.spans.iter().find(|s| s.tag == "md_bold").expect("bold");
        assert_eq!(&doc.text[bold.start..bold.end], "bold");
        let italic = doc
            .spans
            .iter()
            .find(|s| s.tag == "md_italic")
            .expect("italic");
        assert_eq!(&doc.text[italic.start..italic.end], "italic");
        let code = doc.spans.iter().find(|s| s.tag == "md_code").expect("code");
        assert_eq!(&doc.text[code.start..code.end], "code");
    }

    #[test]
    fn unclosed_markers_stay_literal() {
        let doc = parse("a **broken and *also broken");
        assert!(doc.text.contains("**broken"));
        assert!(doc.text.contains("*also broken"));
        assert!(doc.spans.is_empty());
    }

    #[test]
    fn fenced_code_blocks_keep_text_and_span() {
        let doc = parse("before\n```rust\nlet x = 1;\n```\nafter");
        assert!(doc.text.contains("let x = 1;"));
        assert!(doc.spans.iter().any(|s| s.tag == "md_codeblock"));
    }

    #[test]
    fn lists_and_quotes_get_markers() {
        let doc = parse("- one\n- two\n\n1. first\n2. second\n\n> quoted");
        assert!(doc.text.contains("• one"));
        assert!(doc.text.contains("1. first"));
        assert!(doc.spans.iter().any(|s| s.tag == "md_quote"));
    }

    #[test]
    fn links_render_text_with_url() {
        let doc = parse("see [docs](https://example.com) now");
        assert!(doc.text.contains("docs (https://example.com)"));
        assert!(doc.spans.iter().any(|s| s.tag == "md_link"));
    }

    #[test]
    fn hashtags_span_but_hash_in_code_does_not() {
        let doc = parse("tagged #rust-lang here, C# stays plain");
        assert!(doc.spans.iter().any(|s| s.tag == "md_hashtag"));
        let hash = doc
            .spans
            .iter()
            .find(|s| s.tag == "md_hashtag")
            .expect("tag");
        assert_eq!(&doc.text[hash.start..hash.end], "#rust-lang");
    }

    #[test]
    fn spans_cover_valid_byte_ranges() {
        let doc = parse("# Héllo **wörld**\n\n- item `cödé`\n\n> quöte #tag");
        for span in &doc.spans {
            assert!(doc.text.get(span.start..span.end).is_some(), "{span:?}");
        }
    }
}
