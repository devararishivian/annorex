//! Wrapping of the document to the width of the pane.

use std::collections::BTreeMap;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::doc::{Doc, DocSpan, DocTable, SpanSource, SpanStyle, joins};

/// One row of the document view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualLine {
    /// The index of the `DocLine` that this row shows.
    pub doc_line: usize,
    pub spans: Vec<DocSpan>,
    /// True for the first row of the `DocLine`. The gutter shows the line number there.
    pub first: bool,
}

impl VisualLine {
    pub fn plain(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    /// The source offset of each character of the row. `None` for text that has no source.
    pub fn offsets(&self) -> Vec<Option<usize>> {
        let mut out = Vec::new();
        for span in &self.spans {
            match &span.source {
                Some(SpanSource { range, exact: true }) => {
                    out.extend(span.text.char_indices().map(|(index, _)| Some(range.start + index)));
                }
                Some(SpanSource { range, exact: false }) => {
                    out.extend(span.text.chars().map(|_| Some(range.start)));
                }
                None => out.extend(span.text.chars().map(|_| None)),
            }
        }
        out
    }

    /// The index of the character of this row that shows the stop at source `offset`.
    pub fn cell_of(&self, offset: usize) -> Option<usize> {
        let mut position = 0;
        for span in &self.spans {
            let count = span.text.chars().count();
            match &span.source {
                Some(SpanSource { range, exact: true }) if range.contains(&offset) => {
                    let skipped = span
                        .text
                        .char_indices()
                        .take_while(|&(index, _)| range.start + index < offset)
                        .count();
                    return Some(position + skipped);
                }
                Some(SpanSource { range, exact: false }) if range.start == offset && count > 0 => {
                    return Some(position);
                }
                _ => {}
            }
            position += count;
        }
        None
    }
}

/// A glyph: one character and the zero-width characters that join it.
struct Cluster {
    text: String,
    style: SpanStyle,
    width: usize,
    source: Option<SpanSource>,
}

impl Cluster {
    fn is_space(&self) -> bool {
        self.text == " "
    }
}

fn clusters(spans: &[DocSpan]) -> Vec<Cluster> {
    let mut out: Vec<Cluster> = Vec::new();
    let mut previous: Option<char> = None;
    for span in spans {
        for (index, c) in span.text.char_indices() {
            let source = span.source.as_ref().map(|source| {
                if source.exact {
                    let start = source.range.start + index;
                    SpanSource { range: start..start + c.len_utf8(), exact: true }
                } else {
                    source.clone()
                }
            });
            match out.last_mut() {
                Some(last) if joins(previous, c) && last.style == span.style => {
                    last.text.push(c);
                    last.width = last.text.width();
                    join_source(&mut last.source, source);
                }
                _ => out.push(Cluster {
                    text: c.to_string(),
                    style: span.style,
                    width: c.width().unwrap_or(0),
                    source,
                }),
            }
            previous = Some(c);
        }
    }
    out
}

/// Add the source of a joined character to the source of its glyph.
fn join_source(into: &mut Option<SpanSource>, next: Option<SpanSource>) {
    let (Some(first), Some(next)) = (into.as_mut(), next) else { return };
    if first.exact && next.exact && first.range.end == next.range.start {
        first.range.end = next.range.end;
    } else {
        first.exact = false;
        first.range.end = first.range.end.max(next.range.end);
    }
}

/// True when `next` continues `last` in the source, so one span can hold both.
fn continues(last: &DocSpan, cluster: &Cluster) -> bool {
    last.style == cluster.style
        && match (&last.source, &cluster.source) {
            (None, None) => true,
            (Some(a), Some(b)) if a.exact && b.exact => a.range.end == b.range.start,
            (Some(a), Some(b)) => !a.exact && !b.exact && a.range == b.range,
            _ => false,
        }
}

fn spans_of(row: &[Cluster]) -> Vec<DocSpan> {
    let mut spans: Vec<DocSpan> = Vec::new();
    for cluster in row {
        match spans.last_mut() {
            Some(last) if continues(last, cluster) => {
                last.text.push_str(&cluster.text);
                if let (Some(into), Some(next)) = (last.source.as_mut(), cluster.source.as_ref())
                    && into.exact
                {
                    into.range.end = next.range.end;
                }
            }
            _ => spans.push(DocSpan {
                text: cluster.text.clone(),
                style: cluster.style,
                source: cluster.source.clone(),
            }),
        }
    }
    spans
}

/// Break `spans` into rows of at most `width` columns. Break at spaces when possible, else at
/// the width. A glyph wider than the whole width becomes `…`.
fn wrap_spans(spans: &[DocSpan], width: usize) -> Vec<Vec<DocSpan>> {
    let mut rows: Vec<Vec<Cluster>> = vec![Vec::new()];
    let mut used = 0;
    let all = clusters(spans);
    let mut index = 0;
    while let Some(cluster) = all.get(index) {
        if cluster.is_space() {
            if used < width {
                used += 1;
                if let Some(row) = rows.last_mut() {
                    row.push(Cluster {
                        text: " ".into(),
                        style: cluster.style,
                        width: 1,
                        source: cluster.source.clone(),
                    });
                }
            } else {
                rows.push(Vec::new());
                used = 0;
            }
            index += 1;
            continue;
        }
        let word_end = all
            .get(index..)
            .map_or(index, |rest| index + rest.iter().position(Cluster::is_space).unwrap_or(rest.len()));
        let word = all.get(index..word_end).unwrap_or_default();
        let word_width: usize = word.iter().map(|c| c.width).sum();
        if used > 0 && used + word_width > width {
            // The spaces before the break stay in the row: they fit, and a cursor can stand on one.
            rows.push(Vec::new());
            used = 0;
        }
        for cluster in word {
            let (text, cluster_width, source) = if cluster.width > width {
                let source = cluster.source.clone().map(|source| SpanSource { exact: false, ..source });
                ("…".to_owned(), 1, source)
            } else {
                (cluster.text.clone(), cluster.width, cluster.source.clone())
            };
            if used + cluster_width > width {
                rows.push(Vec::new());
                used = 0;
            }
            used += cluster_width;
            if let Some(row) = rows.last_mut() {
                row.push(Cluster { text, style: cluster.style, width: cluster_width, source });
            }
        }
        index = word_end;
    }
    trim_end(&mut rows);
    rows.iter().map(|row| spans_of(row)).collect()
}

fn trim_end(rows: &mut [Vec<Cluster>]) {
    if let Some(row) = rows.last_mut() {
        while row.last().is_some_and(Cluster::is_space) {
            row.pop();
        }
    }
}

/// The most columns that one long word takes in a table column before the columns share the rest.
const WORD_CAP: usize = 20;
/// The text between two table columns.
const COLUMN_GAP: &str = " │ ";

/// The rows of the whole document at `width` columns.
pub fn wrap_doc(doc: &Doc, width: u16) -> Vec<VisualLine> {
    let width = usize::from(width.max(1));
    let mut out = Vec::new();
    let mut doc_line = 0;
    while let Some(line) = doc.lines.get(doc_line) {
        if let Some(table) = doc.tables.iter().find(|table| table.lines.start == doc_line)
            && let Some(rows) = wrap_table(doc, table, width)
        {
            out.extend(rows);
            doc_line = table.lines.end;
            continue;
        }
        if line.is_blank() {
            out.push(VisualLine { doc_line, spans: Vec::new(), first: true });
        } else {
            for (part, spans) in wrap_hanging(&line.spans, width).into_iter().enumerate() {
                out.push(VisualLine { doc_line, spans, first: part == 0 });
            }
        }
        doc_line += 1;
    }
    out
}

/// The rows of one line. The prefix of the line is its first spans without a source: the quote
/// bars, the list marker, or the list indent. The rows after the first start with `continuation`
/// of the prefix, so the text of a list item or a quote lines up under the text of its first row.
// Adapted from plannotator-tui (MIT) at e537abd: the hanging indent and its half-width rule.
fn wrap_hanging(spans: &[DocSpan], width: usize) -> Vec<Vec<DocSpan>> {
    let lead = spans.iter().take_while(|span| span.source.is_none()).count();
    let (prefix, body) = spans.split_at(lead);
    let indent = spans_width(prefix);
    // A narrow pane would leave too little room for the text: then the rows use all of it.
    if indent == 0 || body.is_empty() || indent * 2 > width {
        return wrap_spans(spans, width);
    }
    let rest = continuation(prefix);
    wrap_spans(body, width - indent)
        .into_iter()
        .enumerate()
        .map(|(part, row)| {
            let mut spans = if part == 0 { prefix.to_vec() } else { rest.clone() };
            spans.extend(row);
            spans
        })
        .collect()
}

fn spans_width(spans: &[DocSpan]) -> usize {
    spans.iter().map(|span| span.text.width()).sum()
}

/// The width of the longest word of a cell.
fn longest_word(cell: &[DocSpan]) -> usize {
    let text: String = cell.iter().map(|span| span.text.as_str()).collect();
    text.split_whitespace().map(UnicodeWidthStr::width).max().unwrap_or(0)
}

/// The widths of the columns for `available` columns. Each column keeps at least its longest word
/// (at most `WORD_CAP`). The rest is shared by the natural width. `None` when even the minimum
/// widths do not fit.
fn fit_columns(natural: &[usize], minimum: &[usize], available: usize) -> Option<Vec<usize>> {
    let base: usize = minimum.iter().sum();
    if base > available {
        return None;
    }
    let want: Vec<usize> = natural.iter().zip(minimum).map(|(n, m)| n.saturating_sub(*m)).collect();
    let want_sum: usize = want.iter().sum();
    let extra = available - base;
    let mut give: Vec<usize> =
        want.iter().map(|w| (extra * w).checked_div(want_sum).unwrap_or(0).min(*w)).collect();
    let mut left = extra.saturating_sub(give.iter().sum());
    while left > 0 {
        let mut moved = false;
        for (given, wanted) in give.iter_mut().zip(&want) {
            if left > 0 && *given < *wanted {
                *given += 1;
                left -= 1;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    Some(minimum.iter().zip(&give).map(|(m, g)| m + g).collect())
}

/// `spans` cut to `width` columns.
fn clip_spans(spans: &[DocSpan], width: usize) -> Vec<DocSpan> {
    let mut row = Vec::new();
    let mut used = 0;
    for cluster in clusters(spans) {
        if used + cluster.width > width {
            break;
        }
        used += cluster.width;
        row.push(cluster);
    }
    spans_of(&row)
}

/// The prefix of a row below the first: the same width, without the list marker.
fn continuation(prefix: &[DocSpan]) -> Vec<DocSpan> {
    prefix
        .iter()
        .map(|span| DocSpan {
            text: span.text.chars().map(|c| if c == '│' { c } else { ' ' }).collect(),
            ..span.clone()
        })
        .collect()
}

/// The rows of a table at `width` columns. `None` when the table fits: then the lines of the
/// document show as they are. A table that is too wide gets narrower columns and its cells wrap.
/// If even the narrowest columns do not fit, each line is clipped.
// Adapted from plannotator-tui (MIT): the column widths and the clip fallback.
fn wrap_table(doc: &Doc, table: &DocTable, width: usize) -> Option<Vec<VisualLine>> {
    let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 || table.lines.is_empty() {
        return None;
    }
    let mut natural = vec![0; columns];
    let mut minimum = vec![1; columns];
    for cells in &table.rows {
        for (column, cell) in cells.iter().enumerate() {
            if let (Some(n), Some(m)) = (natural.get_mut(column), minimum.get_mut(column)) {
                *n = (*n).max(spans_width(cell));
                *m = (*m).max(longest_word(cell).min(WORD_CAP));
            }
        }
    }
    let gaps = COLUMN_GAP.width() * (columns - 1);
    let prefix_width = table.prefixes.iter().map(|prefix| spans_width(prefix)).max().unwrap_or(0);
    let budget = width.saturating_sub(prefix_width);
    if natural.iter().sum::<usize>() + gaps <= budget {
        return None;
    }
    let widths = budget.checked_sub(gaps).and_then(|available| fit_columns(&natural, &minimum, available));
    let Some(widths) = widths else {
        let rows = table.lines.clone().filter_map(|index| doc.lines.get(index).map(|line| (index, line)));
        return Some(
            rows.map(|(doc_line, line)| VisualLine {
                doc_line,
                spans: clip_spans(&line.spans, width),
                first: true,
            })
            .collect(),
        );
    };
    let style = SpanStyle {
        table: true,
        quote: doc.lines.get(table.lines.start).is_some_and(|line| line.spans.iter().any(|s| s.style.quote)),
        ..SpanStyle::default()
    };
    let added = |text: String| DocSpan { text, style, source: None };
    let mut out = Vec::new();
    let all: Vec<Vec<Vec<Vec<DocSpan>>>> = table
        .rows
        .iter()
        .map(|cells| {
            widths
                .iter()
                .enumerate()
                .map(|(column, &w)| wrap_spans(cells.get(column).map_or(&[][..], Vec::as_slice), w))
                .collect()
        })
        .collect();
    // When a cell wraps, a rule between the body rows keeps them apart.
    let any_wrap = all.iter().flatten().any(|rows| rows.len() > 1);
    for (row, wrapped) in all.iter().enumerate() {
        let doc_line = table.line_of_row(row);
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let prefix = table.prefixes.get(doc_line - table.lines.start).cloned().unwrap_or_default();
        for part in 0..height {
            let mut spans = if part == 0 { prefix.clone() } else { continuation(&prefix) };
            for (column, &w) in widths.iter().enumerate() {
                let piece =
                    wrapped.get(column).and_then(|rows| rows.get(part)).map_or(&[][..], Vec::as_slice);
                spans.extend(piece.iter().cloned());
                if column + 1 < columns {
                    spans.push(added(" ".repeat(w.saturating_sub(spans_width(piece)))));
                    spans.push(added(COLUMN_GAP.into()));
                }
            }
            out.push(VisualLine { doc_line, spans, first: part == 0 });
        }
        if row + 1 == table.head_rows {
            let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
            let mut spans = prefix.clone();
            spans.push(DocSpan {
                text: rule.join("─┼─"),
                style,
                source: Some(SpanSource { range: table.rule_source.clone(), exact: false }),
            });
            out.push(VisualLine { doc_line: doc_line + 1, spans, first: true });
        } else if any_wrap && row >= table.head_rows && row + 1 < table.rows.len() {
            let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
            let mut spans = continuation(&prefix);
            spans.push(added(rule.join("─┼─")));
            out.push(VisualLine { doc_line, spans, first: false });
        }
    }
    Some(out)
}

/// The rows for each width that was drawn. A resize back to an earlier width costs nothing.
#[derive(Debug, Default)]
pub struct WrapCache {
    rows: BTreeMap<u16, Vec<VisualLine>>,
}

impl WrapCache {
    pub fn get(&mut self, doc: &Doc, width: u16) -> &[VisualLine] {
        self.rows.entry(width).or_insert_with(|| wrap_doc(doc, width))
    }

    /// Forget all rows. Call it when the document changes.
    pub fn clear(&mut self) {
        self.rows.clear();
    }

    /// The widths in the cache, smallest first.
    pub fn widths(&self) -> Vec<u16> {
        self.rows.keys().copied().collect()
    }
}

/// `text` cut to `width` columns. A cut ends with `…`.
pub fn fit(text: &str, width: u16) -> String {
    let width = usize::from(width);
    if text.width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for cluster in clusters(&[DocSpan { text: text.to_owned(), style: SpanStyle::default(), source: None }]) {
        if used + cluster.width > width - 1 {
            break;
        }
        used += cluster.width;
        out.push_str(&cluster.text);
    }
    out.push('…');
    out
}

/// The columns of the line number gutter: the digits of the largest line number, and one space.
pub fn gutter_width(doc: &Doc) -> u16 {
    let largest = doc.lines.iter().map(|line| line.line).max().unwrap_or(1);
    let digits = largest.max(1).ilog10() + 1;
    u16::try_from(digits + 1).unwrap_or(u16::MAX)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use std::fmt::Write as _;

    use unicode_width::UnicodeWidthStr;

    use super::*;
    use crate::doc::Doc;

    fn texts(lines: &[VisualLine]) -> Vec<String> {
        lines.iter().map(|line| line.plain().trim_end().to_owned()).collect()
    }

    #[test]
    fn wraps_at_spaces() {
        let doc = Doc::parse("alpha beta gamma delta\n");
        assert_eq!(texts(&wrap_doc(&doc, 11)), ["alpha beta", "gamma delta"]);
    }

    #[test]
    fn long_token_breaks_at_width() {
        let doc = Doc::parse("see https://example.com/a/very/long/path/without/spaces\n");
        let lines = wrap_doc(&doc, 10);
        assert!(lines.iter().all(|line| line.plain().width() <= 10), "{:?}", texts(&lines));
        assert_eq!(
            texts(&lines).concat().replace(' ', ""),
            "seehttps://example.com/a/very/long/path/without/spaces"
        );
        assert_eq!(texts(&lines)[1], "https://ex");
    }

    #[test]
    fn wide_chars_never_split_or_overflow() {
        let text = "日本語のテキスト 👩‍💻👨‍👩‍👧 é\u{301} mixed 漢字かな\n";
        let doc = Doc::parse(text);
        for width in 1..=12 {
            let lines = wrap_doc(&doc, width);
            for line in &lines {
                let plain = line.plain();
                assert!(plain.width() <= usize::from(width), "width {width}: {plain:?}");
                assert!(!plain.ends_with('\u{200d}'), "a ZWJ sequence was split at width {width}: {plain:?}");
            }
            if width >= 2 {
                let joined = texts(&lines).concat().replace(' ', "");
                assert!(joined.contains("👩‍💻"), "width {width}: {joined:?}");
                assert!(joined.contains("é\u{301}"), "width {width}: {joined:?}");
            }
        }
    }

    #[test]
    fn first_flag_marks_only_the_first_part() {
        let doc = Doc::parse("one two three four\n\nnext\n");
        let lines = wrap_doc(&doc, 8);
        let flags: Vec<(usize, bool)> = lines.iter().map(|line| (line.doc_line, line.first)).collect();
        assert_eq!(texts(&lines)[..3], ["one two", "three", "four"]);
        assert_eq!(flags, [(0, true), (0, false), (0, false), (1, true), (2, true)]);
    }

    #[test]
    fn styles_stay_on_their_text() {
        let doc = Doc::parse("plain **bold words** end\n");
        let lines = wrap_doc(&doc, 11);
        let bold: Vec<&str> = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .filter(|span| span.style.strong)
            .map(|span| span.text.as_str())
            .collect();
        assert_eq!(bold.concat().replace(' ', ""), "boldwords");
    }

    #[test]
    fn wrap_splits_source_ranges() {
        let doc = Doc::parse("alpha beta gamma delta\n");
        let lines = wrap_doc(&doc, 11);
        let ranges: Vec<Vec<(std::ops::Range<usize>, bool)>> = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .filter_map(|s| s.source.as_ref())
                    .map(|s| (s.range.clone(), s.exact))
                    .collect()
            })
            .collect();
        assert_eq!(
            ranges,
            [vec![(0..11, true)], vec![(11..22, true)]],
            "the space before a break stays in its row"
        );
    }

    #[test]
    fn a_broken_long_token_keeps_continuous_ranges() {
        let doc = Doc::parse("abcdefghij\n");
        let lines = wrap_doc(&doc, 4);
        let ranges: Vec<std::ops::Range<usize>> = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .filter_map(|s| s.source.clone())
            .map(|s| s.range)
            .collect();
        assert_eq!(ranges, [0..4, 4..8, 8..10]);
    }

    #[test]
    fn an_inexact_range_stays_on_both_parts() {
        let doc = Doc::parse("```\nab\tcd ef\n```\n");
        let lines = wrap_doc(&doc, 4);
        let sources: Vec<_> =
            lines.iter().flat_map(|line| line.spans.iter()).filter_map(|s| s.source.clone()).collect();
        assert!(sources.len() >= 2);
        assert!(sources.iter().all(|s| !s.exact && s.range == (4..12)), "{sources:?}");
    }

    #[test]
    fn cell_of_finds_the_char_of_a_stop() {
        let doc = Doc::parse("alpha beta gamma delta\n");
        let lines = wrap_doc(&doc, 11);
        assert_eq!(lines[0].cell_of(3), Some(3));
        assert_eq!(lines[1].cell_of(11), Some(0));
        assert_eq!(lines[1].cell_of(3), None);
    }

    #[test]
    fn a_wrapped_list_item_hangs_under_its_text() {
        let doc = Doc::parse("- alpha beta gamma delta epsilon zeta eta theta iota kappa lambda\n");
        let lines = wrap_doc(&doc, 30);
        let rows: Vec<String> = lines.iter().map(VisualLine::plain).collect();
        assert!(rows.len() > 1, "{rows:?}");
        assert!(rows[0].starts_with("- alpha"), "{rows:?}");
        for row in &rows[1..] {
            assert!(row.starts_with("  ") && !row.starts_with("   "), "{rows:?}");
        }
        assert!(rows.iter().all(|row| row.width() <= 30), "{rows:?}");
        for line in &lines[1..] {
            let offsets = line.offsets();
            assert_eq!(offsets[..2], [None, None], "the indent has no source");
            let offset = offsets[2].expect("the text has a source");
            let word = line.plain()[2..].split(' ').next().expect("a word").to_owned();
            assert_eq!(&doc.source[offset..offset + word.len()], word);
            assert_eq!(line.cell_of(offset), Some(2));
        }
    }

    #[test]
    fn a_nested_numbered_item_in_a_quote_hangs_under_its_text() {
        let doc = Doc::parse("> 1. outer\n>    1. one two three four five six seven eight nine ten\n");
        let lines = wrap_doc(&doc, 30);
        let rows: Vec<String> =
            lines.iter().filter(|line| line.doc_line == 1).map(VisualLine::plain).collect();
        assert!(rows.len() > 1, "{rows:?}");
        assert!(rows[0].starts_with("│   1. one"), "{rows:?}");
        for row in &rows[1..] {
            assert!(row.starts_with("│      ") && !row.starts_with("│       "), "{rows:?}");
        }
        assert!(rows.iter().all(|row| row.width() <= 30), "{rows:?}");
    }

    #[test]
    fn a_prefix_wider_than_half_the_width_wraps_as_plain_text() {
        let doc = Doc::parse("> > > > 1. one two three\n");
        let lines = wrap_doc(&doc, 16);
        assert_eq!(texts(&lines), ["│ │ │ │ 1. one", "two three"]);
    }

    #[test]
    fn a_wrapped_quote_keeps_its_bars() {
        let doc = Doc::parse("> alpha beta gamma delta epsilon\n");
        assert_eq!(texts(&wrap_doc(&doc, 16)), ["│ alpha beta", "│ gamma delta", "│ epsilon"]);
    }

    /// A table of 3 columns and 12 rows with long prose in the middle column.
    fn plan_table() -> String {
        let mut text = String::from("| # | Task | Model |\n|---|------|-------|\n");
        for row in 1..=12 {
            let task = "Write the failing test for this step first and then change the code until the whole suite passes again, keeping every other behavior of the viewer exactly as it was";
            let model = if row % 2 == 0 { "sonnet" } else { "opus-with-a-longer-name" };
            let _ = writeln!(text, "| {row} | {task} {row} | {model} |");
        }
        text
    }

    /// The display column of each ` │ ` or ` ─┼─ ` separator in `line`.
    fn separator_columns(line: &str, glyph: char) -> Vec<usize> {
        let mut column = 0;
        let mut out = Vec::new();
        for c in line.chars() {
            if c == glyph {
                out.push(column);
            }
            column += UnicodeWidthChar::width(c).unwrap_or(0);
        }
        out
    }

    #[test]
    fn a_table_that_fits_is_unchanged() {
        let doc = Doc::parse("| a | bb | ccc |\n|---|----|-----|\n| 1 | 22 | 333 |\n");
        let lines = wrap_doc(&doc, 120);
        assert_eq!(texts(&lines), ["a │ bb │ ccc", "──┼────┼────", "1 │ 22 │ 333"]);
        assert_eq!(lines.iter().map(|line| line.doc_line).collect::<Vec<_>>(), [0, 1, 2]);
    }

    #[test]
    fn a_wide_table_wraps_each_cell_in_its_column() {
        let doc = Doc::parse(&plan_table());
        let lines = wrap_doc(&doc, 80);
        assert!(lines.len() > 14, "the long cells wrap onto more rows");
        for line in &lines {
            assert!(line.plain().width() <= 80, "too wide: {:?}", line.plain());
        }
        for doc_line in 0..doc.lines.len() {
            let rows: Vec<String> = lines
                .iter()
                .filter(|line| line.doc_line == doc_line)
                .map(VisualLine::plain)
                .filter(|row| doc_line == 1 || !row.contains('┼'))
                .collect();
            let glyph = if doc_line == 1 { '┼' } else { '│' };
            let columns: Vec<Vec<usize>> = rows.iter().map(|row| separator_columns(row, glyph)).collect();
            assert!(columns.iter().all(|found| found.len() == 2), "line {doc_line}: {rows:?}");
            assert!(columns.windows(2).all(|pair| pair[0] == pair[1]), "line {doc_line}: {rows:?}");
        }
        let rule = lines.iter().find(|line| line.doc_line == 1).expect("rule").plain();
        assert!(rule.contains("─┼─"), "{rule}");
        let rows_of_first: Vec<String> = lines
            .iter()
            .filter(|line| line.doc_line == 2 && !line.plain().contains('┼'))
            .map(VisualLine::plain)
            .collect();
        assert!(rows_of_first.len() > 1);
        let joined: String = rows_of_first
            .iter()
            .map(|row| row.split(" │ ").nth(1).unwrap_or("").trim())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.starts_with("Write the failing test") && joined.ends_with("as it was 1"), "{joined}");
    }

    #[test]
    fn a_long_word_takes_at_most_20_columns_before_it_shares() {
        let word = "x".repeat(60);
        let prose = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi";
        let doc = Doc::parse(&format!("| W | T |\n|---|---|\n| {word} | {prose} |\n"));
        // The minimum widths are 20 and 7. The space for the rest is 50 - 3 - 27 = 20. The first
        // column wants 40 more and the second wants 76 more, so the first gets 7 of the 20.
        let lines = wrap_doc(&doc, 50);
        let first = lines.iter().find(|line| line.doc_line == 2).expect("row").plain();
        let column = separator_columns(&first, '│')[0];
        assert_eq!(column, 20 + 7 + 1, "the long word column has 20 columns and its share: {first:?}");
        assert!(lines.iter().all(|line| line.plain().width() <= 50));
    }

    #[test]
    fn a_table_that_cannot_fit_is_clipped() {
        let doc = Doc::parse(
            "| aaaaaaaaaaaaaaaaaaaa | bbbbbbbbbbbbbbbbbbbb | cccccccccccccccccccc |\n|--|--|--|\n| 1 | 2 | 3 |\n",
        );
        let lines = wrap_doc(&doc, 30);
        assert_eq!(lines.len(), 3, "one row for each line, no wrap");
        assert!(lines.iter().all(|line| line.plain().width() <= 30));
    }

    #[test]
    fn table_cells_keep_their_source_ranges() {
        let doc = Doc::parse(&plan_table());
        for width in [80u16, 120] {
            let lines = wrap_doc(&doc, width);
            let mut found = false;
            for line in lines.iter().filter(|line| line.doc_line == 4) {
                let plain = line.plain();
                if let Some(at) = plain.find("suite") {
                    let column = plain[..at].chars().count();
                    let offset = line.offsets()[column].expect("a cell character has a source");
                    assert_eq!(&doc.source[offset..offset + 5], "suite", "width {width}");
                    found = true;
                }
            }
            assert!(found, "the word is on a row at width {width}");
        }
    }

    #[test]
    fn a_table_in_a_quote_keeps_its_prefix_and_width() {
        let doc = Doc::parse(&plan_table().replace("\n|", "\n> |").replacen('|', "> |", 1));
        let lines = wrap_doc(&doc, 60);
        assert!(lines.len() > 14);
        assert!(lines.iter().all(|line| line.plain().width() <= 60 && line.plain().starts_with("│ ")));
    }

    #[test]
    fn wrapped_body_rows_have_a_rule_between_them() {
        let doc = Doc::parse(&plan_table());
        let lines = wrap_doc(&doc, 80);
        let rules: Vec<&VisualLine> = lines.iter().filter(|line| line.plain().contains("─┼─")).collect();
        assert_eq!(rules.len(), 12, "the header rule and 11 rules between 12 body rows");
        let first_row = lines.iter().find(|line| line.doc_line == 2).expect("row").plain();
        let bars = separator_columns(&first_row, '│');
        for rule in &rules {
            assert_eq!(separator_columns(&rule.plain(), '┼'), bars, "{:?}", rule.plain());
        }
        let between: Vec<&&VisualLine> = rules.iter().filter(|line| line.doc_line > 1).collect();
        assert!(
            between.iter().all(|line| !line.first && line.spans.iter().all(|span| span.source.is_none()))
        );
        assert!(lines.iter().all(|line| line.plain().width() <= 80));
    }

    #[test]
    fn a_reflowed_table_without_wrapping_has_no_extra_rules() {
        let doc = Doc::parse("| a | bb |\n|---|----|\n| 1 | 22 |\n| 3 | 44 |\n");
        let rules = wrap_doc(&doc, 120).iter().filter(|line| line.plain().contains("─┼─")).count();
        assert_eq!(rules, 1);
    }

    #[test]
    fn cache_keeps_one_result_per_width() {
        let doc = Doc::parse("alpha beta gamma delta\n");
        let mut cache = WrapCache::default();
        assert_eq!(cache.get(&doc, 11).len(), 2);
        assert_eq!(cache.get(&doc, 80).len(), 1);
        assert_eq!(cache.widths(), [11, 80]);
        assert_eq!(cache.get(&doc, 11).len(), 2);
        cache.clear();
        assert_eq!(cache.widths(), [] as [u16; 0]);
    }

    #[test]
    fn fit_adds_ellipsis_only_when_cut() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("exactly10!", 10), "exactly10!");
        assert_eq!(fit("longer than ten", 10), "longer th…");
        assert_eq!(fit("日本語テキスト", 7), "日本語…");
        assert_eq!(fit("abc", 0), "");
    }

    #[test]
    fn gutter_width_for_999_and_1000_lines() {
        assert_eq!(gutter_width(&Doc::parse(&"x\n\n".repeat(500))), 4);
        assert_eq!(gutter_width(&Doc::parse(&"x\n\n".repeat(501))), 5);
        assert_eq!(gutter_width(&Doc::parse("")), 2);
    }
}
