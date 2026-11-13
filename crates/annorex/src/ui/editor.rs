//! The text editor for one annotation. It appends text and has no caret movement. It scrolls when the
//! text is longer than the box.

use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
}

impl Editor {
    pub fn new(kind: Kind, range: Range<usize>) -> Self {
        Self { kind, range, text: String::new(), label: None, scroll: None, rows: 0, general: false }
    }

    /// The editor of the general note, with the text that the note has now.
    pub fn general(text: String) -> Self {
        Self { kind: Kind::Comment, range: 0..0, text, label: None, scroll: None, rows: 0, general: true }
    }

    /// The editor that a draft saved.
    pub fn from_editing(editing: Editing) -> Self {
        Self {
            kind: editing.kind,
            range: editing.range,
            text: editing.text,
            label: editing.label,
            scroll: None,
            rows: 0,
            general: editing.general,
        }
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
    /// `PgUp` and `PgDn` scroll. `esc` cancels. Each change of the text follows the end again.
    pub fn handle_key(&mut self, key: KeyEvent) -> EditorResult {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => return EditorResult::Cancel,
            KeyCode::Enter if alt || key.modifiers.contains(KeyModifiers::SHIFT) => self.push('\n'),
            KeyCode::Enter => return EditorResult::Save,
            KeyCode::Char('j') if ctrl => self.push('\n'),
            KeyCode::PageUp => self.scroll_rows(false, MAX_ROWS),
            KeyCode::PageDown => self.scroll_rows(true, MAX_ROWS),
            KeyCode::Backspace => {
                self.text.pop();
                self.scroll = None;
            }
            KeyCode::Char(c) if !ctrl && !alt => self.push(c),
            _ => {}
        }
        EditorResult::Editing
    }

    /// Add text at the end. The box follows the end again.
    pub fn push_str(&mut self, text: &str) {
        self.text.push_str(text);
        self.scroll = None;
    }

    fn push(&mut self, c: char) {
        self.text.push(c);
        self.scroll = None;
    }

    /// Scroll `rows` rows down or up. At the end, the box follows the end of the text again.
    pub fn scroll_rows(&mut self, down: bool, rows: usize) {
        let last = self.rows.saturating_sub(MAX_ROWS);
        let top = self.scroll.unwrap_or(last).min(last);
        let top = if down { top.saturating_add(rows) } else { top.saturating_sub(rows) };
        self.scroll = if top >= last { None } else { Some(top) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_only_comment_saves_without_text() {
        let mut editor = Editor::new(Kind::Comment, 0..1);
        assert_eq!(editor.problem(), Some("write a comment"));
        editor.label = Some("nit".to_owned());
        assert_eq!(editor.problem(), None);
    }
}
