//! The text editor for one annotation. It appends text and has no caret movement.

use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::draft::Editing;
use crate::review::Kind;

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
    pub label: Option<String>,
}

impl Editor {
    pub fn new(kind: Kind, range: Range<usize>) -> Self {
        Self { kind, range, text: String::new(), label: None }
    }

    /// The editor that a draft saved.
    pub fn from_editing(editing: Editing) -> Self {
        Self { kind: editing.kind, range: editing.range, text: editing.text, label: editing.label }
    }

    /// The state that a draft keeps.
    pub fn to_editing(&self) -> Editing {
        Editing {
            kind: self.kind,
            range: self.range.clone(),
            text: self.text.clone(),
            label: self.label.clone(),
        }
    }

    /// Why the annotation cannot be saved, or `None` when it can.
    pub fn problem(&self) -> Option<&'static str> {
        match self.kind {
            Kind::Comment if self.text.trim().is_empty() && self.label.is_none() => {
                Some("write a comment or choose a label with tab")
            }
            Kind::Replace if self.text.trim().is_empty() => Some("write the new text"),
            _ => None,
        }
    }

    /// Handle one key. `enter` saves. `ctrl+j`, `alt+enter`, and `shift+enter` add a new line.
    /// `tab` selects the next label, and after the last label none. `esc` cancels.
    pub fn handle_key(&mut self, key: KeyEvent, labels: &[String]) -> EditorResult {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => return EditorResult::Cancel,
            KeyCode::Enter if alt || key.modifiers.contains(KeyModifiers::SHIFT) => self.text.push('\n'),
            KeyCode::Enter => return EditorResult::Save,
            KeyCode::Char('j') if ctrl => self.text.push('\n'),
            KeyCode::Tab => self.next_label(labels),
            KeyCode::Backspace => {
                self.text.pop();
            }
            KeyCode::Char(c) if !ctrl && !alt => self.text.push(c),
            _ => {}
        }
        EditorResult::Editing
    }

    fn next_label(&mut self, labels: &[String]) {
        self.label = match &self.label {
            None => labels.first().cloned(),
            Some(current) => labels
                .iter()
                .position(|label| label == current)
                .map_or_else(|| labels.first(), |index| labels.get(index + 1))
                .cloned(),
        };
    }
}
