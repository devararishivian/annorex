//! The annotations of one document and their export as Markdown.

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// What an annotation asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Comment,
    Delete,
    Replace,
    /// Approval. It needs no note text.
    LooksGood,
}

/// One note on a range of the source of a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: u64,
    pub kind: Kind,
    /// The bytes of the source that the annotation covers.
    pub range: Range<usize>,
    /// The source text of `range`, when the annotation was made.
    pub quote: String,
    pub note: String,
    /// The new text. `Some` only for `Replace`.
    pub replacement: Option<String>,
    pub label: Option<String>,
    /// True when the quote no longer stands at `range` in the document.
    pub detached: bool,
    /// When a send or a paste took this annotation. A change of the annotation clears it.
    #[serde(default)]
    pub sent_at: Option<String>,
}

impl Annotation {
    /// A comment needs a note or a label. A replace needs a replacement. A delete is always valid.
    pub fn is_valid(&self) -> bool {
        match self.kind {
            Kind::Comment => !self.note.trim().is_empty() || self.label.is_some(),
            Kind::Delete | Kind::LooksGood => true,
            Kind::Replace => self.replacement.as_ref().is_some_and(|text| !text.trim().is_empty()),
        }
    }
}

/// The annotations of one document and the general note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Review {
    pub annotations: Vec<Annotation>,
    /// The general note of "send with a note".
    pub note: String,
    pub next_id: u64,
}

impl Default for Review {
    fn default() -> Self {
        Self { annotations: Vec::new(), note: String::new(), next_id: 1 }
    }
}

impl Review {
    /// Add an annotation on `range` of `source`. The quote is the source text of the range.
    /// Returns the id. An id is never used twice.
    pub fn add(&mut self, source: &str, kind: Kind, range: Range<usize>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.annotations.push(Annotation {
            id,
            kind,
            quote: source.get(range.clone()).unwrap_or_default().to_owned(),
            range,
            note: String::new(),
            replacement: (kind == Kind::Replace).then(String::new),
            label: None,
            detached: false,
            sent_at: None,
        });
        id
    }

    /// Change one annotation. Returns false when there is no such id.
    /// A changed annotation counts as unsent again.
    pub fn update(&mut self, id: u64, edit: impl FnOnce(&mut Annotation)) -> bool {
        self.annotations
            .iter_mut()
            .find(|a| a.id == id)
            .map(|annotation| {
                edit(annotation);
                annotation.sent_at = None;
            })
            .is_some()
    }

    /// The annotations that no send or paste took yet, in document order.
    pub fn unsent(&self) -> Vec<&Annotation> {
        self.in_order().into_iter().filter(|a| a.sent_at.is_none()).collect()
    }

    pub fn has_unsent(&self) -> bool {
        self.annotations.iter().any(|a| a.sent_at.is_none())
    }

    /// A send or a paste took every annotation that it did not take before.
    pub fn mark_sent(&mut self, at: &str) {
        for annotation in self.annotations.iter_mut().filter(|a| a.sent_at.is_none()) {
            annotation.sent_at = Some(at.to_owned());
        }
    }

    pub fn remove(&mut self, id: u64) -> Option<Annotation> {
        let index = self.annotations.iter().position(|a| a.id == id)?;
        Some(self.annotations.remove(index))
    }

    /// The annotations whose range contains `offset`, in document order.
    pub fn at(&self, offset: usize) -> Vec<&Annotation> {
        self.in_order().into_iter().filter(|a| a.range.contains(&offset)).collect()
    }

    /// All annotations by `range.start`, then by id.
    pub fn in_order(&self) -> Vec<&Annotation> {
        let mut out: Vec<&Annotation> = self.annotations.iter().collect();
        out.sort_by_key(|a| (a.range.start, a.id));
        out
    }

    pub fn is_empty(&self) -> bool {
        self.annotations.is_empty()
    }

    /// Check each annotation against the document after a load. When `source[range]` is not the
    /// quote, move the range to the first match of the quote. With no match, set `detached`.
    pub fn reanchor(&mut self, source: &str) {
        for annotation in &mut self.annotations {
            if let Some(range) = reanchor_quote(&annotation.quote, source, &annotation.range) {
                annotation.range = range;
                annotation.detached = false;
            } else {
                annotation.detached = true;
            }
        }
    }
}

/// The place of `range` of `old` in `new`: the same range when the text stands there, else the
/// first match of the text, else `None`.
pub fn reanchor_range(old: &str, new: &str, range: &Range<usize>) -> Option<Range<usize>> {
    reanchor_quote(old.get(range.clone())?, new, range)
}

/// The place of `quote` in `source`: `range` when the quote stands there, else the first match of
/// a non-empty quote, else `None`.
pub fn reanchor_quote(quote: &str, source: &str, range: &Range<usize>) -> Option<Range<usize>> {
    if source.get(range.clone()) == Some(quote) {
        return Some(range.clone());
    }
    if quote.is_empty() {
        return None;
    }
    source.find(quote).map(|start| start..start + quote.len())
}

/// The annotations of one file, or of the last reply, for the export.
#[allow(missing_debug_implementations, reason = "holds a closure")]
pub struct Section<'a> {
    /// The name in the heading: `last reply`, or a path.
    pub name: &'a str,
    pub source: &'a str,
    /// The 1-based source line of a byte offset.
    pub line_of: &'a dyn Fn(usize) -> usize,
    pub annotations: Vec<&'a Annotation>,
}

/// A quote of at most this many characters on one line stands inline.
const INLINE_LIMIT: usize = 60;

/// `word "text"` for a short quote on one line, else `word` and the text in a fence that is longer
/// than any backtick run in it.
fn quoted(word: &str, text: &str) -> String {
    let text = text.strip_suffix('\n').unwrap_or(text);
    if !text.contains('\n') && text.chars().count() <= INLINE_LIMIT {
        return format!("{word} \"{text}\"");
    }
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat((longest + 1).max(3));
    format!("{word}\n{fence}\n{text}\n{fence}")
}

fn block(number: usize, section: &Section<'_>, annotation: &Annotation) -> String {
    let place = if annotation.detached {
        "text not found".to_owned()
    } else {
        let first = (section.line_of)(annotation.range.start);
        let last = (section.line_of)(annotation.range.end.saturating_sub(1).max(annotation.range.start));
        if first == last { format!("line {first}") } else { format!("lines {first}–{last}") }
    };
    let quote = if annotation.detached {
        annotation.quote.as_str()
    } else {
        section.source.get(annotation.range.clone()).unwrap_or(&annotation.quote)
    };
    let mut lines = vec![format!("## Annotation {number} ({place})")];
    match annotation.kind {
        Kind::Comment => lines.push(quoted("Comment on:", quote)),
        Kind::Delete => lines.push(quoted("Remove this:", quote)),
        Kind::LooksGood => lines.push(quoted("Looks good:", quote)),
        Kind::Replace => {
            lines.push(quoted("Replace this:", quote));
            lines.push(quoted("With:", annotation.replacement.as_deref().unwrap_or_default()));
        }
    }
    if let Some(label) = &annotation.label {
        lines.push(format!("Label: {label}"));
    }
    if !annotation.note.trim().is_empty() {
        lines.extend(
            annotation
                .note
                .trim_end()
                .lines()
                .map(|line| if line.is_empty() { ">".to_owned() } else { format!("> {line}") }),
        );
    }
    lines.join("\n")
}

/// The Markdown that goes to the agent: the general note under a `# Note` heading, as a quote,
/// then one section for each file.
///
/// A section lists its annotations in document order, numbered from 1. A section without
/// annotations is left out. The text ends with one line feed. Nothing to export gives `""`.
pub fn export(note: &str, sections: &[Section<'_>]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !note.trim().is_empty() {
        let quote: Vec<String> = note
            .trim()
            .lines()
            .map(|line| if line.is_empty() { ">".to_owned() } else { format!("> {line}") })
            .collect();
        parts.push("# Note".to_owned());
        parts.push(quote.join("\n"));
    }
    for section in sections.iter().filter(|section| !section.annotations.is_empty()) {
        parts.push(format!("# Annotations on {}", section.name));
        let mut ordered = section.annotations.clone();
        ordered.sort_by_key(|a| (a.range.start, a.id));
        for (index, annotation) in ordered.into_iter().enumerate() {
            parts.push(block(index + 1, section, annotation));
        }
    }
    if parts.is_empty() {
        return String::new();
    }
    parts.join("\n\n") + "\n"
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn line_of(source: &str) -> impl Fn(usize) -> usize + '_ {
        move |offset| source[..offset.min(source.len())].matches('\n').count() + 1
    }

    fn numbered(count: usize) -> String {
        (1..=count).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n") + "\n"
    }

    fn export_one(note: &str, source: &str, review: &Review) -> String {
        let line = line_of(source);
        let section = Section { name: "last reply", source, line_of: &line, annotations: review.in_order() };
        export(note, &[section])
    }

    fn lines_range(source: &str, from: usize, to: usize) -> Range<usize> {
        let start: usize = source.split_inclusive('\n').take(from - 1).map(str::len).sum();
        let length: usize =
            source.split_inclusive('\n').skip(from - 1).take(to - from + 1).map(str::len).sum();
        start..start + length - 1
    }

    #[test]
    fn export_matches_the_spec_example() {
        let mut source = numbered(30);
        source = source.replace("line 12", "open the login page now");
        let mut review = Review::default();
        let start = source.find("login page").expect("text");
        let first = review.add(&source, Kind::Comment, start..start + 10);
        review.update(first, |a| a.note = "Which page?".to_owned());
        let second = review.add(&source, Kind::Delete, lines_range(&source, 20, 24));
        review.update(second, |a| a.note = "Not needed.".to_owned());
        let expected = "# Annotations on last reply\n\n\
            ## Annotation 1 (line 12)\n\
            Comment on: \"login page\"\n\
            > Which page?\n\n\
            ## Annotation 2 (lines 20–24)\n\
            Remove this:\n\
            ```\n\
            line 20\nline 21\nline 22\nline 23\nline 24\n\
            ```\n\
            > Not needed.\n";
        assert_eq!(export_one("", &source, &review), expected);
    }

    #[test]
    fn export_of_looks_good() {
        let source = "alpha\n";
        let mut review = Review::default();
        let id = review.add(source, Kind::LooksGood, 0..5);
        let text = export_one("", source, &review);
        assert!(text.contains("## Annotation 1 (line 1)\nLooks good: \"alpha\"\n"), "{text}");
        assert!(!text.contains("> "), "{text}");
        review.update(id, |a| a.note = "fine".to_owned());
        assert!(export_one("", source, &review).ends_with("Looks good: \"alpha\"\n> fine\n"));
        let long = "alpha\nbeta\n";
        let mut review = Review::default();
        review.add(long, Kind::LooksGood, 0..10);
        let text = export_one("", long, &review);
        assert!(text.contains("(lines 1–2)\nLooks good:\n```\nalpha\nbeta\n```"), "{text}");
    }

    #[test]
    fn old_draft_still_reads() {
        let json = r#"{"id":1,"kind":"Comment","range":{"start":0,"end":5},"quote":"alpha","note":"n","replacement":null,"label":null,"detached":false}"#;
        let a: Annotation = serde_json::from_str(json).expect("old annotation");
        assert_eq!(a.kind, Kind::Comment);
        let new = serde_json::to_string(&Kind::LooksGood).expect("json");
        assert_eq!(new, "\"LooksGood\"");
    }

    #[test]
    fn a_range_with_its_line_feed_has_no_blank_line_in_the_fence() {
        let source = numbered(6);
        let mut range = lines_range(&source, 2, 3);
        range.end += 1;
        let mut review = Review::default();
        review.add(&source, Kind::Delete, range);
        let text = export_one("", &source, &review);
        assert!(text.contains("```\nline 2\nline 3\n```\n"), "{text}");
        assert!(text.contains("(lines 2–3)"), "{text}");
    }

    #[test]
    fn replace_and_label_lines() {
        let source = "call foo here\n";
        let mut review = Review::default();
        let id = review.add(source, Kind::Replace, 5..8);
        review.update(id, |a| {
            a.replacement = Some("bar".to_owned());
            a.label = Some("must fix".to_owned());
        });
        let text = export_one("", source, &review);
        assert!(text.contains("Replace this: \"foo\"\nWith: \"bar\"\nLabel: must fix\n"), "{text}");
    }

    #[test]
    fn general_note_comes_first() {
        let source = "alpha\n";
        let mut review = Review::default();
        let id = review.add(source, Kind::Comment, 0..5);
        review.update(id, |a| a.note = "ok".to_owned());
        let text = export_one("Please fix all.", source, &review);
        assert!(text.starts_with("# Note\n\n> Please fix all.\n\n# Annotations on last reply\n"), "{text}");
    }

    #[test]
    fn a_general_note_that_starts_with_a_slash_or_bang_is_quoted_under_a_heading() {
        let source = "alpha\n";
        let mut review = Review::default();
        let id = review.add(source, Kind::Comment, 0..5);
        review.update(id, |a| a.note = "ok".to_owned());
        let text = export_one("!run this\n/second", source, &review);
        assert!(text.starts_with("# Note\n\n> !run this\n> /second\n\n# Annotations on"), "{text}");
    }

    #[test]
    fn a_multi_line_general_note_keeps_its_blank_lines_as_quote_markers() {
        let text = export("one\n\ntwo", &[]);
        assert_eq!(text, "# Note\n\n> one\n>\n> two\n");
    }

    #[test]
    fn fence_is_longer_than_backtick_runs() {
        let source = "a\n`````\nb\n";
        let mut review = Review::default();
        review.add(source, Kind::Delete, 0..source.len() - 1);
        let text = export_one("", source, &review);
        assert!(text.contains("\n``````\na\n`````\nb\n``````\n"), "{text}");
    }

    #[test]
    fn a_long_one_line_quote_is_fenced() {
        let source = format!("{}\n", "x".repeat(61));
        let mut review = Review::default();
        review.add(&source, Kind::Comment, 0..61);
        let text = export_one("", &source, &review);
        assert!(text.contains(&format!("Comment on:\n```\n{}\n```\n", "x".repeat(61))), "{text}");
        let short = format!("{}\n", "x".repeat(60));
        let mut review = Review::default();
        review.add(&short, Kind::Comment, 0..60);
        assert!(export_one("", &short, &review).contains(&format!("Comment on: \"{}\"", "x".repeat(60))));
    }

    #[test]
    fn selection_over_bold_quotes_the_source() {
        let source = "We change the **login** page\n";
        let start = source.find("**login**").expect("bold");
        let mut review = Review::default();
        let id = review.add(source, Kind::Comment, start..start + 9);
        assert_eq!(review.annotations[0].quote, "**login**");
        assert!(export_one("", source, &review).contains("Comment on: \"**login**\""));
        assert_eq!(id, 1);
    }

    #[test]
    fn multi_line_note_quotes_each_line() {
        let source = "alpha\n";
        let mut review = Review::default();
        let id = review.add(source, Kind::Comment, 0..5);
        review.update(id, |a| a.note = "a\n\nb".to_owned());
        assert!(export_one("", source, &review).contains("> a\n>\n> b\n"));
    }

    #[test]
    fn annotations_export_in_document_order() {
        let source = "one two three\n";
        let mut review = Review::default();
        let late = review.add(source, Kind::Comment, 8..13);
        review.update(late, |a| a.note = "late".to_owned());
        let early = review.add(source, Kind::Comment, 0..3);
        review.update(early, |a| a.note = "early".to_owned());
        let text = export_one("", source, &review);
        let early_at = text.find("early").expect("early");
        let late_at = text.find("late").expect("late");
        assert!(early_at < late_at, "{text}");
        assert!(text.contains("## Annotation 1 (line 1)\nComment on: \"one\""), "{text}");
        let ids: Vec<u64> = review.in_order().iter().map(|a| a.id).collect();
        assert_eq!(ids, [early, late]);
    }

    #[test]
    fn same_start_orders_by_id() {
        let source = "alpha\n";
        let mut review = Review::default();
        let a = review.add(source, Kind::Comment, 0..5);
        let b = review.add(source, Kind::Delete, 0..2);
        let ids: Vec<u64> = review.in_order().iter().map(|a| a.id).collect();
        assert_eq!(ids, [a, b]);
    }

    #[test]
    fn at_finds_the_annotations_that_contain_the_offset() {
        let source = "alpha beta\n";
        let mut review = Review::default();
        let a = review.add(source, Kind::Comment, 0..5);
        let b = review.add(source, Kind::Delete, 3..9);
        let ids = |offset| review.at(offset).iter().map(|a| a.id).collect::<Vec<_>>();
        assert_eq!(ids(1), [a]);
        assert_eq!(ids(4), [a, b]);
        assert_eq!(ids(5), [b]);
        assert_eq!(ids(9), Vec::<u64>::new());
    }

    #[test]
    fn remove_and_update_use_the_id() {
        let source = "alpha\n";
        let mut review = Review::default();
        assert!(review.is_empty());
        let id = review.add(source, Kind::Comment, 0..5);
        assert!(!review.update(id + 9, |_| {}));
        assert!(review.remove(id).is_some());
        assert!(review.remove(id).is_none());
        assert!(review.is_empty());
        assert_eq!(review.add(source, Kind::Comment, 0..5), id + 1, "ids are not reused");
    }

    #[test]
    fn validity_follows_the_kind() {
        let source = "alpha\n";
        let mut review = Review::default();
        let comment = review.add(source, Kind::Comment, 0..5);
        let delete = review.add(source, Kind::Delete, 0..5);
        let replace = review.add(source, Kind::Replace, 0..5);
        let valid =
            |review: &Review, id| review.annotations.iter().find(|a| a.id == id).expect("a").is_valid();
        assert!(!valid(&review, comment));
        assert!(valid(&review, delete));
        assert!(!valid(&review, replace));
        review.update(comment, |a| a.label = Some("question".to_owned()));
        review.update(replace, |a| a.replacement = Some("x".to_owned()));
        assert!(valid(&review, comment));
        assert!(valid(&review, replace));
        review.update(replace, |a| a.replacement = Some("  ".to_owned()));
        assert!(!valid(&review, replace));
    }

    #[test]
    fn a_detached_annotation_has_no_line() {
        let source = "alpha\n";
        let mut review = Review::default();
        let id = review.add(source, Kind::Comment, 0..5);
        review.update(id, |a| {
            a.detached = true;
            a.note = "n".to_owned();
        });
        let text = export_one("", "other text\n", &review);
        assert!(text.contains("## Annotation 1 (text not found)\nComment on: \"alpha\""), "{text}");
    }

    #[test]
    fn two_sections_follow_the_given_order_and_empty_ones_are_left_out() {
        let first = "alpha\n";
        let second = "beta\n";
        let mut one = Review::default();
        one.add(first, Kind::Delete, 0..5);
        let mut two = Review::default();
        two.add(second, Kind::Delete, 0..4);
        let empty = Review::default();
        let (la, lb) = (line_of(first), line_of(second));
        let sections = [
            Section { name: "a.md", source: first, line_of: &la, annotations: one.in_order() },
            Section { name: "none.md", source: first, line_of: &la, annotations: empty.in_order() },
            Section { name: "b.md", source: second, line_of: &lb, annotations: two.in_order() },
        ];
        let text = export("", &sections);
        let a = text.find("# Annotations on a.md").expect("a");
        let b = text.find("# Annotations on b.md").expect("b");
        assert!(a < b && !text.contains("none.md"), "{text}");
        assert!(text.contains("## Annotation 1 (line 1)\nRemove this: \"beta\""), "{text}");
    }

    #[test]
    fn nothing_to_export_gives_an_empty_text() {
        assert_eq!(export("", &[]), "");
    }
}
