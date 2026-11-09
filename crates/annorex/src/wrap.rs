//! Wrapping of the document to the width of the pane.

use std::collections::BTreeMap;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::doc::{Doc, DocSpan, SpanStyle};

/// The zero width joiner. A character after it belongs to the same glyph.
const ZWJ: char = '\u{200d}';

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
}

/// A glyph: one character and the zero-width characters that join it.
struct Cluster {
    text: String,
    style: SpanStyle,
    width: usize,
}

impl Cluster {
    fn is_space(&self) -> bool {
        self.text == " "
    }
}

fn clusters(spans: &[DocSpan]) -> Vec<Cluster> {
    let mut out: Vec<Cluster> = Vec::new();
    let mut joined = false;
    for span in spans {
        for c in span.text.chars() {
            let joins = joined || c == ZWJ || c.width().unwrap_or(0) == 0;
            joined = c == ZWJ;
            match out.last_mut() {
                Some(last) if joins && last.style == span.style => {
                    last.text.push(c);
                    last.width = last.text.width();
                }
                _ => out.push(Cluster {
                    text: c.to_string(),
                    style: span.style,
                    width: c.width().unwrap_or(0),
                }),
            }
        }
    }
    out
}

fn spans_of(row: &[Cluster]) -> Vec<DocSpan> {
    let mut spans: Vec<DocSpan> = Vec::new();
    for cluster in row {
        match spans.last_mut() {
            Some(last) if last.style == cluster.style => last.text.push_str(&cluster.text),
            _ => spans.push(DocSpan { text: cluster.text.clone(), style: cluster.style }),
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
                    row.push(Cluster { text: " ".into(), style: cluster.style, width: 1 });
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
            trim_end(&mut rows);
            rows.push(Vec::new());
            used = 0;
        }
        for cluster in word {
            let (text, cluster_width) = if cluster.width > width {
                ("…".to_owned(), 1)
            } else {
                (cluster.text.clone(), cluster.width)
            };
            if used + cluster_width > width {
                rows.push(Vec::new());
                used = 0;
            }
            used += cluster_width;
            if let Some(row) = rows.last_mut() {
                row.push(Cluster { text, style: cluster.style, width: cluster_width });
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
    for cluster in clusters(&[DocSpan { text: text.to_owned(), style: SpanStyle::default() }]) {
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
        lines.iter().map(VisualLine::plain).collect()
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
