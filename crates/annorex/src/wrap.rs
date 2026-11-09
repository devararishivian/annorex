//! Wrapping of the document to the width of the pane.

use std::collections::BTreeMap;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::doc::{Doc, DocSpan, SpanSource, SpanStyle, joins};

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

/// The rows of the whole document at `width` columns.
pub fn wrap_doc(doc: &Doc, width: u16) -> Vec<VisualLine> {
    let width = usize::from(width.max(1));
    let mut out = Vec::new();
    for (doc_line, line) in doc.lines.iter().enumerate() {
        if line.is_blank() {
            out.push(VisualLine { doc_line, spans: Vec::new(), first: true });
            continue;
        }
        for (part, spans) in wrap_spans(&line.spans, width).into_iter().enumerate() {
            out.push(VisualLine { doc_line, spans, first: part == 0 });
        }
    }
    out
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
