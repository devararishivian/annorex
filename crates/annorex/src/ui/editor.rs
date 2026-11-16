//! The text editor for one annotation. It has a caret: the keys and a click move it, and the text goes in
//! at the caret. It scrolls when the text is longer than the box.

use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

use crate::draft::Editing;
use crate::review::Kind;

/// The most text rows that the note box shows.
pub(crate) const MAX_ROWS: usize = 8;
/// The rows that one step of the wheel scrolls.
pub(crate) const WHEEL_ROWS: usize = 3;

/// What the caller does after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorResult {
    /// The editor stays open.
    Editing,
    /// The user asked to save. The caller checks `Editor::problem` first.
    Save,
    Cancel,
}

/// An annotation that the user writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    pub kind: Kind,
    /// The bytes of the source that the annotation covers.
    pub range: Range<usize>,
    /// The note, or for `Replace` the new text.
    pub text: String,
    /// A label that an old annotation or draft has. The box does not show it or change it.
    pub label: Option<String>,
    /// The first row shown. `None` follows the end of the text.
    pub scroll: Option<usize>,
    /// The number of rows of the text in the last frame. The scroll keys use it.
    pub(crate) rows: usize,
    /// True for the general note of "send with a note". It has no range and no label.
    pub general: bool,
    /// The caret: a byte offset in `text` on a character boundary.
    pub(crate) cursor: usize,
    /// The wrap width of the last frame, `0` before the first frame. Up and Down use it.
    pub(crate) width: usize,
}

impl Editor {
    pub fn new(kind: Kind, range: Range<usize>) -> Self {
        Self::with_text(kind, range, String::new(), None, false)
    }

    /// The editor of the general note, with the text that the note has now.
    pub fn general(text: String) -> Self {
        Self::with_text(Kind::Comment, 0..0, text, None, true)
    }

    /// The editor that a draft saved.
    pub fn from_editing(editing: Editing) -> Self {
        Self::with_text(editing.kind, editing.range, editing.text, editing.label, editing.general)
    }

    /// An editor with `text`. The caret is at the end.
    pub(crate) fn with_text(
        kind: Kind,
        range: Range<usize>,
        text: String,
        label: Option<String>,
        general: bool,
    ) -> Self {
        let cursor = text.len();
        Self { kind, range, text, label, scroll: None, rows: 0, general, cursor, width: 0 }
    }

    /// The state that a draft keeps.
    pub fn to_editing(&self) -> Editing {
        Editing {
            kind: self.kind,
            range: self.range.clone(),
            text: self.text.clone(),
            label: self.label.clone(),
            general: self.general,
            annotation: None,
            quote: None,
        }
    }

    /// Why the annotation cannot be saved, or `None` when it can.
    pub fn problem(&self) -> Option<&'static str> {
        if self.general {
            return None;
        }
        match self.kind {
            Kind::Comment if self.text.trim().is_empty() && self.label.is_none() => Some("write a comment"),
            Kind::Replace if self.text.trim().is_empty() => Some("write the new text"),
            _ => None,
        }
    }

    /// Handle one key. `enter` saves. `ctrl+j`, `alt+enter`, and `shift+enter` add a new line.
    /// `PgUp` and `PgDn` scroll. `esc` cancels. The other keys edit at the caret or move it, and the box
    /// follows the caret again.
    // Adapted from plannotator-tui (MIT): the keys and the word rules of the comment box.
    pub fn handle_key(&mut self, key: KeyEvent) -> EditorResult {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => return EditorResult::Cancel,
            KeyCode::Enter if alt || key.modifiers.contains(KeyModifiers::SHIFT) => self.insert('\n'),
            KeyCode::Enter => return EditorResult::Save,
            KeyCode::Char('j' | 'J') if ctrl => self.insert('\n'),
            KeyCode::PageUp => return self.scrolled(false),
            KeyCode::PageDown => return self.scrolled(true),
            KeyCode::Char('a' | 'A') if ctrl => self.cursor = self.line_start(),
            KeyCode::Char('e' | 'E') if ctrl => self.cursor = self.line_end(),
            KeyCode::Char('u' | 'U') if ctrl => self.delete_to(self.line_start()),
            KeyCode::Char('k' | 'K') if ctrl => self.delete_to(self.line_end()),
            // Many terminals send ctrl+h for ctrl+backspace.
            KeyCode::Char('w' | 'W' | 'h' | 'H') if ctrl => self.delete_to(self.previous_word_start()),
            KeyCode::Backspace if ctrl || alt => self.delete_to(self.previous_word_start()),
            KeyCode::Char('d' | 'D') if alt => self.delete_to(self.next_word_end()),
            KeyCode::Delete if ctrl => self.delete_to(self.next_word_end()),
            KeyCode::Char('b' | 'B') if alt => self.cursor = self.previous_word_start(),
            KeyCode::Char('f' | 'F') if alt => self.cursor = self.next_word_end(),
            KeyCode::Backspace => self.delete_to(self.before(self.cursor)),
            KeyCode::Delete => self.delete_to(self.after(self.cursor)),
            KeyCode::Left if ctrl || alt => self.cursor = self.previous_word_start(),
            KeyCode::Right if ctrl || alt => self.cursor = self.next_word_end(),
            KeyCode::Left => self.cursor = self.before(self.cursor),
            KeyCode::Right => self.cursor = self.after(self.cursor),
            KeyCode::Up => self.move_vertical(false),
            KeyCode::Down => self.move_vertical(true),
            KeyCode::Home => self.cursor = self.line_start(),
            KeyCode::End => self.cursor = self.line_end(),
            KeyCode::Char(c) if !ctrl && !alt => self.insert(c),
            _ => return EditorResult::Editing,
        }
        self.scroll = None;
        EditorResult::Editing
    }

    fn scrolled(&mut self, down: bool) -> EditorResult {
        self.scroll_rows(down, MAX_ROWS);
        EditorResult::Editing
    }

    /// Insert text at the caret, for a paste. `\r\n` and `\r` become `\n`. The box follows the caret again.
    pub fn insert_str(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.text.insert_str(self.cursor, &text);
        self.cursor += text.len();
        self.scroll = None;
    }

    fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Remove the text between the caret and `index`. The caret goes to the gap.
    fn delete_to(&mut self, index: usize) {
        let range = self.cursor.min(index)..self.cursor.max(index);
        self.cursor = range.start;
        self.text.replace_range(range, "");
    }

    /// The boundary of the character before `index`, or `0`.
    fn before(&self, index: usize) -> usize {
        self.text.get(..index).and_then(|head| head.char_indices().next_back()).map_or(0, |(at, _)| at)
    }

    /// The boundary after the character at `index`, or `index` at the end.
    fn after(&self, index: usize) -> usize {
        self.text.get(index..).and_then(|tail| tail.chars().next()).map_or(index, |c| index + c.len_utf8())
    }

    fn char_before(&self, index: usize) -> Option<char> {
        self.text.get(..index).and_then(|head| head.chars().next_back())
    }

    fn char_at(&self, index: usize) -> Option<char> {
        self.text.get(index..).and_then(|tail| tail.chars().next())
    }

    /// Where the line of the caret starts.
    fn line_start(&self) -> usize {
        self.text.get(..self.cursor).and_then(|head| head.rfind('\n')).map_or(0, |at| at + 1)
    }

    /// Where the line of the caret ends, before its new line.
    fn line_end(&self) -> usize {
        self.text
            .get(self.cursor..)
            .and_then(|tail| tail.find('\n'))
            .map_or(self.text.len(), |at| self.cursor + at)
    }

    /// Where the word before the caret starts, after the separators between.
    fn previous_word_start(&self) -> usize {
        let mut index = self.cursor;
        while self.char_before(index).is_some_and(|c| !is_word(c)) {
            index = self.before(index);
        }
        while self.char_before(index).is_some_and(is_word) {
            index = self.before(index);
        }
        index
    }

    /// Where the word after the caret ends, after the separators between.
    fn next_word_end(&self) -> usize {
        let mut index = self.cursor;
        while self.char_at(index).is_some_and(|c| !is_word(c)) {
            index = self.after(index);
        }
        while self.char_at(index).is_some_and(is_word) {
            index = self.after(index);
        }
        index
    }

    /// The display row and column of the caret at `width`. At a break, the caret shows at the start of
    /// the later row, next to the character that it is before.
    pub(crate) fn caret(&self, width: usize) -> (usize, usize) {
        let rows = rows(&self.text, width);
        let row = rows.iter().rposition(|range| range.start <= self.cursor).unwrap_or(0);
        let start = rows.get(row).map_or(0, |range| range.start);
        (row, self.text.get(start..self.cursor).map_or(0, text_width))
    }

    /// Move the caret to the display `row` and `column` at `width`, to the nearest character. A row after
    /// the last row puts the caret at the end.
    pub(crate) fn place(&mut self, row: usize, column: usize, width: usize) {
        let rows = rows(&self.text, width);
        let Some(target) = rows.get(row) else {
            self.cursor = self.text.len();
            self.scroll = None;
            return;
        };
        let mut cursor = target.start;
        let mut used = 0;
        for c in self.text.get(target.clone()).unwrap_or_default().chars() {
            let w = c.width().unwrap_or(0);
            if used + w > column {
                break;
            }
            used += w;
            cursor += c.len_utf8();
        }
        // The end of a broken row is the start of the next row, which shows on that next row.
        let breaks = rows.get(row + 1).is_some_and(|next| next.start == target.end);
        self.cursor = if breaks && cursor == target.end { self.before(cursor) } else { cursor };
        self.scroll = None;
    }

    /// Up and Down: the same column on the display row above or below. The first and the last row keep
    /// the caret.
    fn move_vertical(&mut self, down: bool) {
        if self.width == 0 {
            return;
        }
        let (row, column) = self.caret(self.width);
        let Some(target) = (if down { row.checked_add(1) } else { row.checked_sub(1) }) else { return };
        if target < rows(&self.text, self.width).len() {
            self.place(target, column, self.width);
        }
    }

    /// Scroll `rows` rows down or up. At the end, the box follows the end of the text again.
    pub fn scroll_rows(&mut self, down: bool, rows: usize) {
        let last = self.rows.saturating_sub(MAX_ROWS);
        let top = self.scroll.unwrap_or(last).min(last);
        let top = if down { top.saturating_add(rows) } else { top.saturating_sub(rows) };
        self.scroll = if top >= last { None } else { Some(top) };
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn text_width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// The byte range of each display row of `text` at `width` columns. A new line starts a row, and a long
/// line breaks at the column. Each character is on a row. A `\n` is on no row.
pub(crate) fn rows(text: &str, width: usize) -> Vec<Range<usize>> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let (mut start, mut used) = (0, 0);
    for (at, c) in text.char_indices() {
        if c == '\n' {
            rows.push(start..at);
            start = at + 1;
            used = 0;
            continue;
        }
        let w = c.width().unwrap_or(0);
        if used + w > width && at > start {
            rows.push(start..at);
            start = at;
            used = 0;
        }
        used += w;
    }
    rows.push(start..text.len());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(editor: &mut Editor, code: KeyCode, modifiers: KeyModifiers) -> EditorResult {
        editor.handle_key(KeyEvent::new(code, modifiers))
    }

    fn plain(editor: &mut Editor, code: KeyCode) -> EditorResult {
        key(editor, code, KeyModifiers::NONE)
    }

    fn typed(text: &str) -> Editor {
        let mut editor = Editor::new(Kind::Comment, 0..1);
        for c in text.chars() {
            plain(&mut editor, KeyCode::Char(c));
        }
        editor
    }

    /// An editor with `text` and the caret at the byte `cursor`.
    fn at(text: &str, cursor: usize) -> Editor {
        let mut editor = typed(text);
        editor.cursor = cursor;
        editor
    }

    fn state(editor: &Editor) -> (&str, usize) {
        (editor.text.as_str(), editor.cursor)
    }

    const CTRL: KeyModifiers = KeyModifiers::CONTROL;
    const ALT: KeyModifiers = KeyModifiers::ALT;

    #[test]
    fn left_then_type_inserts_in_the_middle() {
        let mut editor = typed("ac");
        plain(&mut editor, KeyCode::Left);
        plain(&mut editor, KeyCode::Char('b'));
        assert_eq!(state(&editor), ("abc", 2));
    }

    #[test]
    fn backspace_and_delete_work_at_the_caret() {
        let mut editor = typed("abcd");
        plain(&mut editor, KeyCode::Left);
        plain(&mut editor, KeyCode::Left);
        plain(&mut editor, KeyCode::Backspace);
        assert_eq!(state(&editor), ("acd", 1));
        plain(&mut editor, KeyCode::Delete);
        assert_eq!(state(&editor), ("ad", 1));
    }

    #[test]
    fn home_end_and_ctrl_a_e_use_the_line() {
        let mut editor = typed("ab");
        key(&mut editor, KeyCode::Char('j'), CTRL);
        for c in "cd".chars() {
            plain(&mut editor, KeyCode::Char(c));
        }
        plain(&mut editor, KeyCode::Home);
        assert_eq!(editor.cursor, 3);
        key(&mut editor, KeyCode::Char('e'), CTRL);
        assert_eq!(editor.cursor, 5);
        key(&mut editor, KeyCode::Char('a'), CTRL);
        assert_eq!(editor.cursor, 3);
        plain(&mut editor, KeyCode::End);
        assert_eq!(editor.cursor, 5);
    }

    #[test]
    fn word_moves_and_word_deletes() {
        let mut editor = typed("foo bar_baz qux");
        key(&mut editor, KeyCode::Left, ALT);
        assert_eq!(editor.cursor, 12);
        key(&mut editor, KeyCode::Left, CTRL);
        assert_eq!(editor.cursor, 4);
        key(&mut editor, KeyCode::Char('f'), ALT);
        assert_eq!(editor.cursor, 11);
        key(&mut editor, KeyCode::Char('w'), CTRL);
        assert_eq!(state(&editor), ("foo  qux", 4));
        key(&mut editor, KeyCode::Right, CTRL);
        assert_eq!(editor.cursor, 8);
        key(&mut editor, KeyCode::Char('b'), ALT);
        assert_eq!(editor.cursor, 5);
        key(&mut editor, KeyCode::Char('d'), ALT);
        assert_eq!(state(&editor), ("foo  ", 5));
        for (code, modifiers) in
            [(KeyCode::Backspace, ALT), (KeyCode::Backspace, CTRL), (KeyCode::Char('h'), CTRL)]
        {
            let mut editor = typed("one two");
            key(&mut editor, code, modifiers);
            assert_eq!(state(&editor), ("one ", 4), "{code:?} {modifiers:?}");
        }
        let mut editor = at("one two", 0);
        key(&mut editor, KeyCode::Delete, CTRL);
        assert_eq!(state(&editor), (" two", 0));
    }

    #[test]
    fn ctrl_u_and_ctrl_k_delete_to_the_line_edges() {
        let mut editor = at("abcdef", 3);
        key(&mut editor, KeyCode::Char('k'), CTRL);
        assert_eq!(state(&editor), ("abc", 3));
        let mut editor = at("abcdef", 3);
        key(&mut editor, KeyCode::Char('u'), CTRL);
        assert_eq!(state(&editor), ("def", 0));
    }

    #[test]
    fn multibyte_text_keeps_the_caret_on_a_boundary() {
        let mut editor = typed("é中x");
        for code in [KeyCode::Left, KeyCode::Left] {
            plain(&mut editor, code);
            assert!(editor.text.is_char_boundary(editor.cursor));
        }
        plain(&mut editor, KeyCode::Char('y'));
        assert_eq!(editor.text, "éy中x");
        for code in [KeyCode::Right, KeyCode::Backspace, KeyCode::Left, KeyCode::Delete] {
            plain(&mut editor, code);
            assert!(editor.text.is_char_boundary(editor.cursor), "{code:?}");
        }
        assert_eq!(editor.text, "éx");
    }

    #[test]
    fn rows_break_at_the_column_and_at_new_lines() {
        assert_eq!(rows("abcdef\ngh", 4), vec![0..4, 4..6, 7..9]);
        assert_eq!(rows("中中中", 4), vec![0..6, 6..9]);
        assert_eq!(rows("", 4), vec![0..0]);
    }

    #[test]
    fn the_caret_at_a_break_shows_on_the_next_row() {
        assert_eq!(at("abcdef", 4).caret(4), (1, 0));
        assert_eq!(at("abcd", 4).caret(4), (0, 4));
        assert_eq!(at("a中b", 4).caret(4), (0, 3));
    }

    #[test]
    fn up_and_down_move_by_display_row() {
        let mut editor = at("abcdefgh", 6);
        editor.width = 4;
        plain(&mut editor, KeyCode::Up);
        assert_eq!(editor.cursor, 2);
        plain(&mut editor, KeyCode::Down);
        assert_eq!(editor.cursor, 6);
        plain(&mut editor, KeyCode::Down);
        assert_eq!(editor.cursor, 6, "the last row has no row below");
        let mut editor = at("ab\ncd", 4);
        editor.width = 10;
        plain(&mut editor, KeyCode::Up);
        assert_eq!(editor.cursor, 1);
    }

    #[test]
    fn insert_str_goes_in_at_the_caret() {
        let mut editor = at("ad", 1);
        editor.insert_str("b\r\nc");
        assert_eq!(state(&editor), ("ab\ncd", 4));
    }

    #[test]
    fn place_moves_to_the_row_and_column() {
        let mut editor = at("abcdefgh", 0);
        editor.place(1, 2, 4);
        assert_eq!(editor.cursor, 6);
        editor.place(9, 0, 4);
        assert_eq!(editor.cursor, 8);
        let mut editor = at("ab\ncd", 0);
        editor.place(0, 7, 10);
        assert_eq!(editor.cursor, 2, "right of the text is the end of the row");
    }

    #[test]
    fn the_old_keys_keep_their_meaning() {
        let mut editor = typed("ab");
        assert_eq!(plain(&mut editor, KeyCode::Enter), EditorResult::Save);
        assert_eq!(plain(&mut editor, KeyCode::Esc), EditorResult::Cancel);
        key(&mut editor, KeyCode::Char('j'), CTRL);
        key(&mut editor, KeyCode::Enter, ALT);
        key(&mut editor, KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(editor.text, "ab\n\n\n");
        plain(&mut editor, KeyCode::PageUp);
        plain(&mut editor, KeyCode::PageDown);
        assert_eq!(state(&editor), ("ab\n\n\n", 5));
    }

    #[test]
    fn an_old_note_opens_with_the_caret_at_the_end() {
        assert_eq!(Editor::general("note".to_owned()).cursor, 4);
    }

    #[test]
    fn a_label_only_comment_saves_without_text() {
        let mut editor = Editor::new(Kind::Comment, 0..1);
        assert_eq!(editor.problem(), Some("write a comment"));
        editor.label = Some("nit".to_owned());
        assert_eq!(editor.problem(), None);
    }
}
