//! Markdown display: a reply becomes styled lines, and each line keeps a map to its source.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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

/// Where the text of a span comes from in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanSource {
    /// The bytes in `Doc::source` that the span shows.
    pub range: Range<usize>,
    /// True when the text is the same as the source bytes. Then character `i` of the text maps to
    /// its own bytes. When false (a tab, an entity, an escape), all characters map to `range`.
    pub exact: bool,
}

/// Text with one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocSpan {
    pub text: String,
    pub style: SpanStyle,
    /// `None` for text that the display adds: list markers, quote bars, and table lines.
    pub source: Option<SpanSource>,
}

/// The zero width joiner. A character after it belongs to the same glyph.
const ZWJ: char = '\u{200d}';

/// True when `c` does not start a new glyph: it joins to the character before it.
pub(crate) fn joins(previous: Option<char>, c: char) -> bool {
    previous == Some(ZWJ) || c == ZWJ || c.width() == Some(0)
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

/// How a file shows: as Markdown or as plain text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Markdown,
    Plain,
}

impl DocKind {
    /// Files with the extension `md` or `markdown` (any case) show as Markdown. All others are plain.
    pub fn of(path: &std::path::Path) -> Self {
        let markdown = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"));
        if markdown { Self::Markdown } else { Self::Plain }
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

    /// Show a text as it is: one `DocLine` for each source line, with the exact source of each
    /// character. A tab shows as spaces up to a width of 4. It is one stop that covers the tab.
    pub fn plain(source: &str) -> Self {
        let source = sanitize(source);
        let line_starts = line_starts(&source);
        let mut lines = Vec::new();
        for (index, &start) in line_starts.iter().enumerate() {
            let end =
                source.get(start..).and_then(|rest| rest.find('\n')).map_or(source.len(), |at| start + at);
            if start == source.len() && index > 0 {
                break; // the empty "line" after the final line feed
            }
            let text = source.get(start..end).unwrap_or_default();
            let mut spans = Vec::new();
            let mut offset = start;
            for (position, piece) in text.split('\t').enumerate() {
                if position > 0 {
                    spans.push(DocSpan {
                        text: TAB.to_owned(),
                        style: SpanStyle::default(),
                        source: Some(SpanSource { range: offset..offset + 1, exact: false }),
                    });
                    offset += 1;
                }
                if !piece.is_empty() {
                    spans.push(DocSpan {
                        text: piece.to_owned(),
                        style: SpanStyle::default(),
                        source: Some(SpanSource { range: offset..offset + piece.len(), exact: true }),
                    });
                    offset += piece.len();
                }
            }
            let range = if spans.is_empty() { start..start } else { start..end };
            lines.push(DocLine { spans, source: range, line: index + 1 });
        }
        Self { source, lines, line_starts }
    }

    /// Render a text as the kind says.
    pub fn for_kind(kind: DocKind, source: &str) -> Self {
        match kind {
            DocKind::Markdown => Self::parse(source),
            DocKind::Plain => Self::plain(source),
        }
    }

    /// The 1-based source line of a byte offset.
    pub fn line_of(&self, offset: usize) -> usize {
        line_of(&self.line_starts, offset)
    }

    /// The number of source lines.
    pub fn source_lines(&self) -> usize {
        self.line_starts.len()
    }

    /// The cursor stops of a `DocLine` with their display columns, in display order. A stop is
    /// the source byte offset of an exact character, or the start of an inexact span. A character
    /// that joins the one before it has no stop.
    fn stop_columns(&self, line: usize) -> Vec<(usize, usize)> {
        let Some(line) = self.lines.get(line) else { return Vec::new() };
        let mut out = Vec::new();
        let mut column = 0;
        for span in &line.spans {
            match &span.source {
                Some(SpanSource { range, exact: true }) => {
                    let mut previous = None;
                    for (index, c) in span.text.char_indices() {
                        if !joins(previous, c) {
                            out.push((range.start + index, column));
                        }
                        column += c.width().unwrap_or(0);
                        previous = Some(c);
                    }
                }
                Some(SpanSource { range, exact: false }) => {
                    if !span.text.is_empty() {
                        out.push((range.start, column));
                    }
                    column += span.text.width();
                }
                None => column += span.text.width(),
            }
        }
        out
    }

    /// The cursor positions of a `DocLine`, as source byte offsets, in display order.
    pub fn stops(&self, line: usize) -> Vec<usize> {
        self.stop_columns(line).into_iter().map(|(offset, _)| offset).collect()
    }

    /// The stop of a `DocLine` at the display column `column`: the last stop that starts at or
    /// before the column, else the first stop.
    pub fn stop_near(&self, line: usize, column: usize) -> Option<usize> {
        let stops = self.stop_columns(line);
        stops.iter().rev().find(|&&(_, at)| at <= column).or(stops.first()).map(|&(offset, _)| offset)
    }

    /// The `DocLine` index and the display column of the stop at `offset`. An offset that is not
    /// a stop gives the last text line that starts at or before it, and column 0.
    pub fn locate(&self, offset: usize) -> (usize, usize) {
        for line in 0..self.lines.len() {
            if let Some((_, column)) = self.stop_columns(line).into_iter().find(|&(stop, _)| stop == offset) {
                return (line, column);
            }
        }
        let line = self
            .lines
            .iter()
            .enumerate()
            .rev()
            .find(|(_, line)| !line.is_blank() && line.source.start <= offset)
            .map_or(0, |(index, _)| index);
        (line, 0)
    }

    /// The end byte of the character at `offset`. A stop of an inexact span ends where the span
    /// ends. A character with joining characters after it ends after them.
    pub fn char_end(&self, offset: usize) -> usize {
        let inexact =
            self.lines.iter().flat_map(|line| line.spans.iter()).find_map(|span| match &span.source {
                Some(SpanSource { range, exact: false }) if range.start == offset => Some(range.end),
                _ => None,
            });
        if let Some(end) = inexact {
            return end;
        }
        let Some(rest) = self.source.get(offset..) else { return self.source.len() };
        let mut chars = rest.char_indices();
        let Some((_, first)) = chars.next() else { return offset };
        let mut end = first.len_utf8();
        let mut previous = first;
        for (index, c) in chars {
            if !joins(Some(previous), c) {
                break;
            }
            end = index + c.len_utf8();
            previous = c;
        }
        offset + end
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
            vec![DocSpan { text: prefix, style: self.prefix_style(), source: None }]
        };
        self.pending = Some(Pending { spans, source: start..start });
    }

    fn push(&mut self, text: &str, style: SpanStyle, range: &Range<usize>) {
        self.begin(range.start);
        if let Some(pending) = &mut self.pending {
            pending.source.end = pending.source.end.max(range.end);
            if !text.is_empty() || style.code_block {
                let text = text.replace('\t', TAB);
                let exact = self.source.get(range.clone()) == Some(text.as_str());
                let source = Some(SpanSource { range: range.clone(), exact });
                pending.spans.push(DocSpan { text, style, source });
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

    #[test]
    fn plain_doc_keeps_source_lines_and_tabs() {
        let doc = Doc::plain("one\n\tindented\n\nlast\n");
        let plains: Vec<String> = doc.lines.iter().map(DocLine::plain).collect();
        assert_eq!(plains, ["one", "    indented", "", "last"]);
        let numbers: Vec<usize> = doc.lines.iter().map(|line| line.line).collect();
        assert_eq!(numbers, [1, 2, 3, 4]);
        assert!(doc.lines[2].is_blank());
        // The tab is one stop at its own byte, and `char_end` covers only that byte.
        assert_eq!(doc.stops(1)[0], 4);
        assert_eq!(doc.char_end(4), 5);
        assert_eq!(doc.stops(1)[1], 5, "the next stop is `i` after the tab");
        assert_eq!(doc.line_of(5), 2);
    }

    #[test]
    fn plain_doc_without_final_line_feed() {
        let doc = Doc::plain("alpha\nbeta");
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[1].source, 6..10);
        assert_eq!(Doc::plain("").lines.len(), 1, "an empty file has one blank line");
    }

    #[test]
    fn plain_doc_maps_every_character() {
        let source = "naïve café\t日本語 👨\u{200d}👩\n";
        let doc = Doc::plain(source);
        let line = &doc.lines[0];
        for span in &line.spans {
            let found = span.source.as_ref().expect("every plain span has a source");
            if found.exact {
                assert_eq!(source.get(found.range.clone()), Some(span.text.as_str()));
            }
        }
        let stops = doc.stops(0);
        assert!(stops.iter().all(|&stop| source.is_char_boundary(stop)), "{stops:?}");
        let last = *stops.last().expect("stop");
        assert_eq!(doc.char_end(last), source.len() - 1, "the joined emoji is one stop");
    }

    #[test]
    fn crlf_file_gives_the_right_line_numbers() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join("w.txt"), "one\r\ntwo\r\nthree\r\n").expect("write");
        let loaded = crate::files::load(dir.path(), std::path::Path::new("w.txt")).expect("load");
        let doc = Doc::plain(&loaded.text);
        assert_eq!(doc.source, "one\ntwo\nthree\n");
        let three = doc.source.find("three").expect("three");
        assert_eq!(doc.line_of(three), 3);
    }

    #[test]
    fn kind_follows_the_extension() {
        use std::path::Path;
        for md in ["A.MD", "x.markdown", "dir/readme.Md"] {
            assert_eq!(DocKind::of(Path::new(md)), DocKind::Markdown, "{md}");
        }
        for plain in ["x.rs", "Makefile", "notes.md.txt", ".md"] {
            assert_eq!(DocKind::of(Path::new(plain)), DocKind::Plain, "{plain}");
        }
        assert_eq!(Doc::for_kind(DocKind::Plain, "# not a heading\n").lines[0].plain(), "# not a heading");
        assert_eq!(Doc::for_kind(DocKind::Markdown, "# heading\n").lines[0].plain(), "heading");
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
    fn exact_spans_map_each_char() {
        let doc = Doc::parse("We change the **login** page\n");
        let stops = doc.stops(0);
        let plain = doc.lines[0].plain();
        assert_eq!(stops.len(), plain.chars().count());
        for (stop, c) in stops.iter().zip(plain.chars()) {
            assert_eq!(doc.source[*stop..].chars().next(), Some(c), "stop {stop}");
        }
        let l = doc.source.find("login").expect("login");
        assert!(stops.contains(&l) && stops.contains(&(l + 1)));
        assert_eq!(doc.locate(l), (0, 14));
        assert_eq!(doc.char_end(l), l + 1);
    }

    #[test]
    fn inexact_span_has_one_stop() {
        let doc = Doc::parse("```\na\tb\n```\n");
        assert_eq!(plains(&doc), ["a    b"]);
        assert_eq!(doc.stops(0), [4]);
        assert_eq!(doc.locate(4), (0, 0));
        assert_eq!(doc.char_end(4), 7);
    }

    #[test]
    fn an_entity_is_one_stop_that_covers_the_entity() {
        let doc = Doc::parse("a &amp; b\n");
        assert_eq!(plains(&doc), ["a & b"]);
        let amp = doc.source.find("&amp;").expect("entity");
        assert!(doc.stops(0).contains(&amp));
        assert_eq!(doc.char_end(amp), amp + 5);
    }

    #[test]
    fn a_joined_glyph_is_one_stop() {
        let doc = Doc::parse("e\u{301}x 👩\u{200d}💻 y\n");
        let stops = doc.stops(0);
        let plain = doc.lines[0].plain();
        assert_eq!(stops.len(), 6, "{plain:?}: {stops:?}");
        assert_eq!(doc.char_end(0), "e\u{301}".len());
    }

    #[test]
    fn prefixes_have_no_stops() {
        let doc = Doc::parse("- item\n");
        assert_eq!(plains(&doc), ["- item"]);
        assert_eq!(doc.stops(0).len(), 4);
        assert_eq!(doc.locate(doc.stops(0)[0]), (0, 2));
    }

    #[test]
    fn source_line_of_offset() {
        let doc = Doc::parse("a\nb\n\nc\n");
        assert_eq!(doc.line_of(0), 1);
        assert_eq!(doc.line_of(2), 2);
        assert_eq!(doc.line_of(5), 4);
    }
}
