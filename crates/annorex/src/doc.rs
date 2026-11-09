//! Markdown display: a reply becomes styled lines, and each line keeps a map to its source.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use unicode_width::UnicodeWidthStr;

use crate::payload::sanitize;

/// The width of a tab in the display.
const TAB: &str = "    ";

/// How a span looks. The UI maps it to colors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "each flag is one independent Markdown style")]
pub struct SpanStyle {
    pub heading: Option<u8>,
    pub strong: bool,
    pub emphasis: bool,
    pub code: bool,
    pub code_block: bool,
    pub strike: bool,
    pub link: bool,
    pub quote: bool,
    pub table: bool,
    pub rule: bool,
}

/// Text with one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocSpan {
    pub text: String,
    pub style: SpanStyle,
}

/// One displayed line, before wrapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocLine {
    pub spans: Vec<DocSpan>,
    /// The byte range in `Doc::source` that the line shows.
    pub source: Range<usize>,
    /// The 1-based source line of `source.start`.
    pub line: usize,
}

impl DocLine {
    /// The text of the line without styles.
    pub fn plain(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    /// True for the empty line between blocks.
    pub fn is_blank(&self) -> bool {
        self.spans.is_empty()
    }
}

/// A rendered document and its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    /// The source after `payload::sanitize`. All ranges point into this text.
    pub source: String,
    pub lines: Vec<DocLine>,
    line_starts: Vec<usize>,
}

impl Doc {
    /// Render Markdown. Control characters are removed first, so none reaches the terminal.
    pub fn parse(source: &str) -> Self {
        let source = sanitize(source);
        let line_starts = line_starts(&source);
        let lines = {
            let mut builder = Builder::new(&source, &line_starts);
            let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
            for (event, range) in Parser::new_ext(&source, options).into_offset_iter() {
                builder.event(event, range);
            }
            builder.finish()
        };
        Self { source, lines, line_starts }
    }

    /// The 1-based source line of a byte offset.
    pub fn line_of(&self, offset: usize) -> usize {
        line_of(&self.line_starts, offset)
    }

    /// The number of source lines.
    pub fn source_lines(&self) -> usize {
        self.line_starts.len()
    }
}

fn line_starts(source: &str) -> Vec<usize> {
    std::iter::once(0).chain(source.match_indices('\n').map(|(index, _)| index + 1)).collect()
}

fn line_of(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|&start| start <= offset).max(1)
}

struct Pending {
    spans: Vec<DocSpan>,
    source: Range<usize>,
}

#[derive(Default)]
struct Table {
    rows: Vec<(Vec<String>, Range<usize>)>,
    head_rows: usize,
    cells: Vec<String>,
    row_source: Range<usize>,
}

struct Builder<'a> {
    source: &'a str,
    line_starts: &'a [usize],
    lines: Vec<DocLine>,
    pending: Option<Pending>,
    style: SpanStyle,
    /// The next number of each open list; `None` for a bullet list.
    lists: Vec<Option<u64>>,
    item_marker: Option<String>,
    quote_depth: usize,
    table: Option<Table>,
}

impl<'a> Builder<'a> {
    fn new(source: &'a str, line_starts: &'a [usize]) -> Self {
        Self {
            source,
            line_starts,
            lines: Vec::new(),
            pending: None,
            style: SpanStyle::default(),
            lists: Vec::new(),
            item_marker: None,
            quote_depth: 0,
            table: None,
        }
    }

    fn prefix_style(&self) -> SpanStyle {
        SpanStyle { quote: self.quote_depth > 0, ..SpanStyle::default() }
    }

    fn text_style(&self) -> SpanStyle {
        SpanStyle { quote: self.quote_depth > 0, ..self.style }
    }

    /// Start a line at `start` if none is open, with the quote and list prefixes.
    fn begin(&mut self, start: usize) {
        if self.pending.is_some() {
            return;
        }
        let mut prefix = "│ ".repeat(self.quote_depth);
        let depth = self.lists.len();
        match self.item_marker.take() {
            Some(marker) => prefix.push_str(&marker),
            None if depth > 0 => prefix.push_str(&"  ".repeat(depth)),
            None => {}
        }
        let spans = if prefix.is_empty() {
            Vec::new()
        } else {
            vec![DocSpan { text: prefix, style: self.prefix_style() }]
        };
        self.pending = Some(Pending { spans, source: start..start });
    }

    fn push(&mut self, text: &str, style: SpanStyle, range: &Range<usize>) {
        self.begin(range.start);
        if let Some(pending) = &mut self.pending {
            pending.source.end = pending.source.end.max(range.end);
            if !text.is_empty() || style.code_block {
                pending.spans.push(DocSpan { text: text.replace('\t', TAB), style });
            }
        }
    }

    fn flush(&mut self) {
        let Some(pending) = self.pending.take() else { return };
        let mut end = pending.source.end.max(pending.source.start);
        while end > pending.source.start && self.source.as_bytes().get(end - 1) == Some(&b'\n') {
            end -= 1;
        }
        let line = line_of(self.line_starts, pending.source.start);
        self.lines.push(DocLine { spans: pending.spans, source: pending.source.start..end, line });
    }

    /// End the block: one blank line, never two.
    fn blank(&mut self) {
        self.flush();
        if self.lines.last().is_some_and(|line| !line.is_blank()) {
            let end = self.lines.last().map_or(0, |line| line.source.end);
            let line = line_of(self.line_starts, end);
            self.lines.push(DocLine { spans: Vec::new(), source: end..end, line });
        }
    }

    /// Text that keeps its line breaks: code blocks and HTML blocks.
    fn push_lines(&mut self, text: &str, range: &Range<usize>) {
        let exact = self.source.get(range.clone()) == Some(text);
        let mut offset = 0;
        for piece in text.split_inclusive('\n') {
            let content = piece.strip_suffix('\n').unwrap_or(piece);
            let start = if exact { range.start + offset } else { range.start };
            let end = if exact { start + content.len() } else { range.end };
            self.push(content, self.text_style(), &(start..end));
            if piece.ends_with('\n') {
                self.flush();
            }
            offset += piece.len();
        }
    }

    #[allow(clippy::too_many_lines, reason = "one match over the Markdown events")]
    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        if self.table.is_some() && self.table_event(&event, &range) {
            return;
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    self.flush();
                    self.style.heading = Some(level as u8);
                    self.begin(range.start);
                    if let Some(pending) = &mut self.pending {
                        pending.source.end = range.end;
                    }
                }
                Tag::BlockQuote(_) => {
                    self.flush();
                    self.quote_depth += 1;
                }
                Tag::CodeBlock(_) => {
                    self.flush();
                    self.style.code_block = true;
                }
                Tag::HtmlBlock => self.flush(),
                Tag::List(start) => {
                    self.flush();
                    self.lists.push(start);
                }
                Tag::Item => {
                    self.flush();
                    let depth = self.lists.len().max(1);
                    let marker = match self.lists.last_mut() {
                        Some(Some(number)) => {
                            let marker = format!("{number}. ");
                            *number += 1;
                            marker
                        }
                        _ => "- ".to_owned(),
                    };
                    self.item_marker = Some(format!("{}{marker}", "  ".repeat(depth - 1)));
                    self.begin(range.start);
                }
                Tag::Emphasis => self.style.emphasis = true,
                Tag::Strong => self.style.strong = true,
                Tag::Strikethrough => self.style.strike = true,
                Tag::Link { .. } | Tag::Image { .. } => self.style.link = true,
                Tag::Table(_) => {
                    self.flush();
                    self.table = Some(Table::default());
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    if self.lists.is_empty() {
                        self.blank();
                    } else {
                        self.flush();
                    }
                }
                TagEnd::Heading(_) => {
                    self.flush();
                    self.style.heading = None;
                    self.blank();
                }
                TagEnd::BlockQuote(_) => {
                    self.flush();
                    self.quote_depth = self.quote_depth.saturating_sub(1);
                    if self.quote_depth == 0 {
                        self.blank();
                    }
                }
                TagEnd::CodeBlock => {
                    self.flush();
                    self.style.code_block = false;
                    self.blank();
                }
                TagEnd::HtmlBlock => self.blank(),
                TagEnd::List(_) => {
                    self.flush();
                    self.lists.pop();
                    if self.lists.is_empty() {
                        self.blank();
                    }
                }
                TagEnd::Item => self.flush(),
                TagEnd::Emphasis => self.style.emphasis = false,
                TagEnd::Strong => self.style.strong = false,
                TagEnd::Strikethrough => self.style.strike = false,
                TagEnd::Link | TagEnd::Image => self.style.link = false,
                _ => {}
            },
            Event::Text(text) => {
                if self.style.code_block {
                    self.push_lines(&text, &range);
                } else {
                    self.push(&text, self.text_style(), &range);
                }
            }
            Event::Code(text) => self.push(&text, SpanStyle { code: true, ..self.text_style() }, &range),
            Event::Html(text) => self.push_lines(&text, &range),
            Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::FootnoteReference(text) => self.push(&text, self.text_style(), &range),
            Event::SoftBreak => self.push(" ", self.text_style(), &range),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.flush();
                self.push("───", SpanStyle { rule: true, ..self.prefix_style() }, &range);
                self.blank();
            }
            Event::TaskListMarker(done) => {
                self.push(if done { "[x] " } else { "[ ] " }, self.text_style(), &range);
            }
        }
    }

    /// Collect a table. Returns true when the event belonged to the table.
    fn table_event(&mut self, event: &Event<'_>, range: &Range<usize>) -> bool {
        let Some(table) = &mut self.table else { return false };
        match event {
            Event::Start(Tag::TableHead | Tag::TableRow) => {
                table.cells.clear();
                table.row_source = range.clone();
            }
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                let cells = std::mem::take(&mut table.cells);
                table.rows.push((cells, table.row_source.clone()));
                if matches!(event, Event::End(TagEnd::TableHead)) {
                    table.head_rows = table.rows.len();
                }
            }
            Event::Start(Tag::TableCell) => table.cells.push(String::new()),
            Event::Text(text) | Event::Code(text) | Event::InlineHtml(text) => {
                if let Some(cell) = table.cells.last_mut() {
                    cell.push_str(&text.replace('\t', TAB));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(cell) = table.cells.last_mut() {
                    cell.push(' ');
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(table) = self.table.take() {
                    self.emit_table(&table);
                }
                self.blank();
            }
            _ => {}
        }
        true
    }

    fn emit_table(&mut self, table: &Table) {
        let columns = table.rows.iter().map(|(cells, _)| cells.len()).max().unwrap_or(0);
        let mut widths = vec![0; columns];
        for (cells, _) in &table.rows {
            for (width, cell) in widths.iter_mut().zip(cells) {
                *width = (*width).max(cell.width());
            }
        }
        let style = SpanStyle { table: true, quote: self.quote_depth > 0, ..SpanStyle::default() };
        for (index, (cells, source)) in table.rows.iter().enumerate() {
            let text: Vec<String> = widths
                .iter()
                .enumerate()
                .map(|(column, width)| {
                    let cell = cells.get(column).map_or("", String::as_str);
                    format!("{cell}{}", " ".repeat(width.saturating_sub(cell.width())))
                })
                .collect();
            self.push(text.join(" │ ").trim_end(), style, source);
            self.flush();
            if index + 1 == table.head_rows {
                let rule: Vec<String> = widths.iter().map(|width| "─".repeat(*width)).collect();
                let line = line_of(self.line_starts, source.start) + 1;
                let start = self.line_starts.get(line - 1).copied().unwrap_or(source.end);
                let end = self.line_starts.get(line).map_or(self.source.len(), |next| next - 1).max(start);
                self.push(&rule.join("─┼─"), style, &(start..end));
                self.flush();
            }
        }
    }

    fn finish(mut self) -> Vec<DocLine> {
        self.flush();
        while self.lines.last().is_some_and(DocLine::is_blank) {
            self.lines.pop();
        }
        self.lines
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn plains(doc: &Doc) -> Vec<String> {
        doc.lines.iter().map(DocLine::plain).collect()
    }

    fn find_line<'a>(doc: &'a Doc, plain: &str) -> &'a DocLine {
        doc.lines.iter().find(|line| line.plain() == plain).unwrap_or_else(|| panic_missing(doc, plain))
    }

    #[allow(clippy::panic, reason = "test helper")]
    fn panic_missing(doc: &Doc, plain: &str) -> ! {
        panic!("no line {plain:?} in {:?}", plains(doc))
    }

    #[test]
    fn heading_maps_to_its_source_line() {
        let doc = Doc::parse("intro\n\n## Plan\n\ntext\n");
        let heading = find_line(&doc, "Plan");
        assert_eq!(heading.line, 3);
        assert_eq!(heading.spans[0].style.heading, Some(2));
        assert_eq!(&doc.source[heading.source.clone()], "## Plan");
        assert_eq!(find_line(&doc, "text").line, 5);
    }

    #[test]
    fn blocks_have_one_blank_line_between_and_none_at_the_end() {
        let doc = Doc::parse("# A\n\n\n\npara\n");
        assert_eq!(plains(&doc), ["A", "", "para"]);
    }

    #[test]
    fn paragraph_soft_breaks_join() {
        let doc = Doc::parse("one\ntwo\n");
        assert_eq!(plains(&doc), ["one two"]);
        assert_eq!(doc.lines[0].line, 1);
    }

    #[test]
    fn nested_and_numbered_lists_indent() {
        let doc = Doc::parse("1. a\n2. b\n   - c\n   - d\n");
        assert_eq!(plains(&doc), ["1. a", "2. b", "  - c", "  - d"]);
        let numbers: Vec<usize> = doc.lines.iter().map(|line| line.line).collect();
        assert_eq!(numbers, [1, 2, 3, 4]);
    }

    #[test]
    fn task_list_shows_the_box() {
        let doc = Doc::parse("- [x] done\n- [ ] open\n");
        assert_eq!(plains(&doc), ["- [x] done", "- [ ] open"]);
    }

    #[test]
    fn code_block_keeps_each_line_and_line_number() {
        let doc = Doc::parse("text\n\n```rust\nfn a() {}\n\tlet x;\n\n```\n");
        assert_eq!(plains(&doc), ["text", "", "fn a() {}", "    let x;", ""]);
        let code = find_line(&doc, "fn a() {}");
        assert_eq!(code.line, 4);
        assert!(code.spans[0].style.code_block);
        assert_eq!(find_line(&doc, "    let x;").line, 5);
        assert_eq!(&doc.source[code.source.clone()], "fn a() {}");
    }

    #[test]
    fn table_cells_align_by_display_width() {
        let doc = Doc::parse("| a | 名前 |\n|---|---|\n| bbb | c |\n");
        assert_eq!(plains(&doc), ["a   │ 名前", "────┼─────", "bbb │ c"]);
        let numbers: Vec<usize> = doc.lines.iter().map(|line| line.line).collect();
        assert_eq!(numbers, [1, 2, 3]);
        assert!(doc.lines.iter().all(|line| line.spans.iter().all(|span| span.style.table)));
    }

    #[test]
    fn emphasis_strong_code_and_strike_styles() {
        let doc = Doc::parse("a *b* **c** `d` ~~e~~ [f](https://x)\n");
        let style = |text: &str| {
            doc.lines[0].spans.iter().find(|span| span.text == text).map(|span| span.style).expect(text)
        };
        assert!(style("b").emphasis);
        assert!(style("c").strong);
        assert!(style("d").code);
        assert!(style("e").strike);
        assert!(style("f").link);
        assert!(!style("a ").emphasis);
        assert_eq!(doc.lines[0].plain(), "a b c d e f");
    }

    #[test]
    fn quote_and_rule() {
        let doc = Doc::parse("> said\n\n---\n");
        assert_eq!(plains(&doc), ["│ said", "", "───"]);
        assert!(doc.lines[0].spans.iter().all(|span| span.style.quote));
        assert!(doc.lines[2].spans[0].style.rule);
    }

    #[test]
    fn escape_bytes_in_reply_are_removed() {
        let doc = Doc::parse("a\u{1b}[31mred\u{7}\r\nnext\n");
        let text = plains(&doc).join("\n");
        assert!(!text.chars().any(char::is_control), "{text:?}");
        assert_eq!(text, "a[31mred next");
    }

    #[test]
    fn empty_reply_gives_no_lines() {
        assert_eq!(Doc::parse("").lines, []);
        assert_eq!(Doc::parse("  \n\n").lines, []);
    }

    #[test]
    fn real_reply_lines_are_in_source_order() {
        let doc = Doc::parse(include_str!("../tests/fixtures/payload-markdown.md"));
        assert!(doc.lines.len() > 10);
        let numbers: Vec<usize> = doc.lines.iter().map(|line| line.line).collect();
        assert!(numbers.windows(2).all(|pair| pair[0] <= pair[1]), "{numbers:?}");
        assert!(numbers.iter().all(|&line| line >= 1 && line <= doc.source_lines()));
        assert!(doc.lines.iter().all(|line| doc.source.get(line.source.clone()).is_some()));
    }

    #[test]
    fn source_line_of_offset() {
        let doc = Doc::parse("a\nb\n\nc\n");
        assert_eq!(doc.line_of(0), 1);
        assert_eq!(doc.line_of(2), 2);
        assert_eq!(doc.line_of(5), 4);
    }
}
