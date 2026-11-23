//! The mouse: a helper for the keys. Each mouse action has a key, and most clicks call the code of
//! that key. A click finds its target in the hit map of the last frame. The ids and the indices in
//! the map can be stale, so each one is checked against the state before use.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use unicode_width::UnicodeWidthStr;

use super::app::{App, BoxView, Effect, Focus, ReplySource, Screen, Selection, is_word};
use super::focus::{Mode, Move};
use super::hits::Hit;
use crate::deliver::DeliveryMode;
use crate::doc::joins;
use crate::keys::Action;
use crate::review::Kind;

/// Two presses at one cell within this time are a double click.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The number of lines that one step of the wheel scrolls.
pub const WHEEL_LINES: usize = 3;

/// The scroll that the wheel gave each part. A part with `None` follows its cursor. Any key ends the
/// scroll, so the view goes back to the focus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Scroll {
    /// True when the wheel moved the document: the view does not follow the focus.
    pub(crate) doc: bool,
    /// The first row of the tree.
    pub(crate) tree: Option<usize>,
    /// The first item of the annotation list.
    pub(crate) notes: Option<usize>,
    /// The first row of the list in the centered box or in the session picker.
    pub(crate) boxed: Option<usize>,
}

/// The first row of each list in the last frame. The wheel starts from it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Tops {
    pub(crate) tree: usize,
    pub(crate) notes: usize,
    pub(crate) boxed: usize,
}

/// The part that the wheel scrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Doc,
    Tree,
    Notes,
    Box,
}

impl App {
    /// Handle one mouse event. A question or the find input takes no mouse event. The help screen
    /// closes on any button. An open editor takes only its `save` and `cancel` buttons.
    pub fn handle_mouse(&mut self, event: MouseEvent) -> Effect {
        if self.confirm.is_some() || self.find_input.is_some() {
            return Effect::None;
        }
        let at = (event.column, event.row);
        if self.help_open {
            if matches!(
                event.kind,
                MouseEventKind::Down(_) | MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            ) {
                self.help_open = false;
            }
            return Effect::None;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => self.press(at),
            MouseEventKind::Drag(MouseButton::Left) => {
                self.drag_to(at);
                Effect::None
            }
            MouseEventKind::Up(MouseButton::Left) => {
                // A drag that selected text is finished: the action strip opens.
                if self.drag.take().is_some() && self.selection.is_some() {
                    self.bar_open = true;
                }
                Effect::None
            }
            MouseEventKind::ScrollDown => {
                self.wheel(at, true);
                Effect::None
            }
            MouseEventKind::ScrollUp => {
                self.wheel(at, false);
                Effect::None
            }
            _ => Effect::None,
        }
    }

    /// True while the wheel holds the view of the document, so the view does not follow the focus.
    pub fn doc_scrolled(&self) -> bool {
        self.scroll.doc
    }

    /// A press of the left button.
    fn press(&mut self, (column, row): (u16, u16)) -> Effect {
        let hit = self.hits.at(column, row).copied();
        let double = self
            .last_click
            .take()
            .is_some_and(|(time, x, y)| (x, y) == (column, row) && time.elapsed() <= DOUBLE_CLICK);
        self.drag = None;
        self.status = None;
        // A press closes the action strip, except on the strip itself. A press on an item acts on
        // the selection first.
        if !matches!(hit, Some(Hit::Bar | Hit::Action(_))) {
            self.bar_open = false;
        }
        if let Some(editor) = self.editor.as_mut() {
            return match hit {
                Some(Hit::EditorSave) => self.editor_done(super::editor::EditorResult::Save),
                Some(Hit::EditorCancel) => self.editor_done(super::editor::EditorResult::Cancel),
                // A click on the text places the caret.
                _ if editor.area.contains(ratatui::layout::Position::new(column, row)) => {
                    let (row, column) = (row - editor.area.y, column - editor.area.x);
                    let width = editor.width;
                    editor.place(editor.top + usize::from(row), usize::from(column), width);
                    Effect::None
                }
                // A click outside the editor does nothing, so no text is lost.
                _ => Effect::None,
            };
        }
        match &self.screen {
            Screen::Error(_) => return hit.and_then(|hit| self.hint(hit)).unwrap_or(Effect::None),
            Screen::Picker(candidates) => {
                let count = candidates.len();
                return match hit {
                    Some(Hit::ListRow(index)) if index < count => {
                        self.picker_index = index;
                        Effect::PickSession(index)
                    }
                    _ => hit.and_then(|hit| self.hint(hit)).unwrap_or(Effect::None),
                };
            }
            Screen::Viewer => {}
        }
        if let Some(view) = self.boxed {
            // The box takes the click. A click outside the box closes it, also on a footer hint or on
            // the send button.
            match hit {
                Some(Hit::ListRow(index)) => return self.box_row(view, index),
                Some(Hit::BoxInside) => {}
                // A stray click must not end the start list, because `esc` there quits.
                _ if self.start_box => {}
                _ => self.close_box(),
            }
            return Effect::None;
        }
        let Some(hit) = hit else { return Effect::None };
        if let Some(effect) = self.hint(hit) {
            return effect;
        }
        match hit {
            Hit::Block(index) if index < self.doc.blocks.len() => self.focus_block(index),
            Hit::DocCell { row: visual, col: text_x } => {
                let Some(offset) = self.offset_at(visual, column.saturating_sub(text_x)) else {
                    return Effect::None;
                };
                if double && self.select_word(offset) {
                    return Effect::None;
                }
                let index = self.doc.block_near(offset).unwrap_or(0);
                self.focus_block(index);
                self.drag = Some(offset);
                self.last_click = Some((Instant::now(), column, row));
            }
            Hit::Action(kind) if self.focus == Focus::Doc => {
                self.bar_open = false;
                return if matches!(kind, Kind::LooksGood | Kind::Delete) {
                    self.add_instant(kind)
                } else {
                    self.begin_new(kind)
                };
            }
            Hit::Card(id) => self.focus_note(id),
            // The same as the edit and the remove key on the note: edit opens the editor, remove asks first.
            Hit::CardEdit(id) | Hit::CardRemove(id) if self.has_note(id) => {
                self.focus = Focus::Doc;
                return self.act_on_note(id, matches!(hit, Hit::CardRemove(_)));
            }
            Hit::Send => return self.request_send(DeliveryMode::Send),
            Hit::TreeRow(index) => return self.tree_row(index),
            Hit::ListRow(index) => {
                let id = self.review().and_then(|review| review.in_order().get(index).map(|a| a.id));
                if let Some(id) = id {
                    self.focus_note(id);
                }
            }
            // The empty part of the action bar or of a side part, the cover of an overlay, or a stale editor button.
            _ => {}
        }
        Effect::None
    }

    /// A click on a footer hint: the same as the key of its action. `None` for another hit.
    fn hint(&mut self, hit: Hit) -> Option<Effect> {
        let Hit::Hint(action) = hit else { return None };
        // The arrow keys always move, so the moves work also when the user unbound their letters.
        let key = match action {
            Action::Down => KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            Action::Up => KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            Action::Left => KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            Action::Right => KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            _ => {
                let chord = self.config.keys.chord(action)?;
                KeyEvent::new(chord.code, chord.modifiers)
            }
        };
        Some(self.handle_key(key))
    }

    /// A row of the centered box: open the reply, choose the session, or open the file of the finder.
    fn box_row(&mut self, view: BoxView, index: usize) -> Effect {
        match view {
            BoxView::Finder => {
                let Some(path) = self.finder.results().nth(index).map(std::path::PathBuf::from) else {
                    return Effect::None;
                };
                self.close_box();
                self.open_path(&path)
            }
            BoxView::Replies => {
                let count = match &self.reply_source {
                    ReplySource::Pick(candidates) => candidates.len(),
                    _ => self.replies.len(),
                };
                if index >= count {
                    return Effect::None;
                }
                if matches!(self.reply_source, ReplySource::Pick(_)) {
                    self.picker_index = index;
                } else {
                    self.panel_index = index;
                }
                self.replies_enter()
            }
        }
    }

    /// A row of the tree: open the file, or open and close the folder.
    fn tree_row(&mut self, index: usize) -> Effect {
        if index >= self.tree.visible().len() {
            return Effect::None;
        }
        self.tree.index = index;
        if self.tree.selected().is_some_and(|entry| entry.is_dir) {
            self.tree.toggle();
            // The focus goes to the tree. A filter stays: the row indices belong to it.
            self.focus = Focus::Tree;
            return Effect::None;
        }
        self.tree_enter()
    }

    /// True when the document on the screen has the note `id`.
    fn has_note(&self, id: u64) -> bool {
        self.review().is_some_and(|review| review.annotations.iter().any(|a| a.id == id))
    }

    /// Focus a block in block mode, as the keys do.
    fn focus_block(&mut self, index: usize) {
        self.focus = Focus::Doc;
        self.selection = None;
        self.scroll.doc = false;
        self.mode = Mode::Block(index);
        self.block_move(Move::Show);
    }

    /// Focus a note and its block: the click on a card or on a row of the annotation list.
    fn focus_note(&mut self, id: u64) {
        let Some(review) = self.review() else { return };
        let Some(index) = review.in_order().iter().position(|a| a.id == id) else { return };
        let place =
            review.annotations.iter().find(|a| a.id == id).filter(|a| !a.detached).map(|a| a.range.start);
        self.annotation_index = index;
        self.focus = Focus::Doc;
        match place {
            Some(start) => {
                let block = self.doc.block_near(start).unwrap_or(0);
                self.focus_block(block);
            }
            None => self.status = Some("the text of this annotation was not found".to_owned()),
        }
    }

    /// The cursor stop under a cell of the document text: `row` is the index of the visual row, and
    /// `column` is the column in the text area. A cell after the end of the text gives the last stop.
    fn offset_at(&mut self, row: usize, column: u16) -> Option<usize> {
        let width = if self.width == 0 { u16::MAX } else { self.width };
        let visual = self.cache.get(&self.doc, width).get(row)?.clone();
        let column = usize::from(column);
        // Measure by glyph, as the renderer does: a joined sequence is one glyph with one width.
        let mut glyphs: Vec<(String, Option<usize>)> = Vec::new();
        let mut previous = None;
        let mut last = None;
        for (c, offset) in visual.plain().chars().zip(visual.offsets()) {
            last = offset.or(last);
            match glyphs.last_mut() {
                Some((text, _)) if joins(previous, c) => text.push(c),
                _ => glyphs.push((c.to_string(), offset.or(last))),
            }
            previous = Some(c);
        }
        // A cell without source inside the row (table padding or a border) keeps the offset of the
        // text on its left. The quote bars and the list marker or indent come before any text: they
        // give the first text of the row.
        let first = glyphs.iter().find_map(|(_, offset)| *offset);
        let mut x = 0;
        let mut found = None;
        for (text, offset) in &glyphs {
            let size = text.width();
            if size > 0 && column < x + size {
                found = offset.or(first);
                break;
            }
            x += size;
        }
        let line = visual.doc_line;
        let stops = self.doc.stops(line);
        let snapped = match found.or(last) {
            Some(offset) => stops.iter().rev().find(|&&stop| stop <= offset).or(stops.first()).copied(),
            None => stops.first().copied(),
        };
        snapped.or_else(|| self.line_start(line))
    }

    /// The double click: select the word at `offset` in text mode. False when no word is there.
    fn select_word(&mut self, offset: usize) -> bool {
        let line = self.doc.locate(offset).0;
        let stops = self.doc.stops(line);
        let source = self.doc.source.as_str();
        let word =
            |stop: &usize| source.get(*stop..).and_then(|rest| rest.chars().next()).is_some_and(is_word);
        let Some(at) = stops.iter().position(|&stop| stop == offset) else { return false };
        let (before, after) = stops.split_at(at);
        // `after` starts with the stop at `offset`.
        let run: Vec<usize> = after.iter().take_while(|stop| word(stop)).copied().collect();
        let (Some(&start_here), Some(&last)) = (run.first(), run.last()) else { return false };
        let first = before.iter().rev().take_while(|stop| word(stop)).last().copied().unwrap_or(start_here);
        self.focus = Focus::Doc;
        self.scroll.doc = false;
        self.mode = Mode::Text;
        self.selection = Some(Selection::Chars { anchor: first });
        self.cursor = last;
        self.bar_open = true;
        true
    }

    /// A drag of the left button. A drag that started in the document text selects characters in
    /// text mode. At the top or the bottom row of the document, each event scrolls by one line.
    fn drag_to(&mut self, (column, row): (u16, u16)) {
        let Some(anchor) = self.drag else { return };
        let area = self.text_area;
        if area.width == 0 || area.height == 0 {
            return;
        }
        let (bottom, right) = (area.bottom() - 1, area.right() - 1);
        if row <= area.y {
            self.wheel_doc(false, 1);
        } else if row >= bottom {
            self.wheel_doc(true, 1);
        }
        let row = row.clamp(area.y, bottom) - area.y;
        let column = column.clamp(area.x, right) - area.x;
        let top = self.view_top_row();
        let Some(offset) = self.offset_at(top + usize::from(row), column) else { return };
        // A press and a release at one cell stay a click: the selection starts when the pointer moves.
        if offset == anchor && self.selection.is_none() {
            return;
        }
        self.last_click = None;
        self.focus = Focus::Doc;
        self.scroll.doc = false;
        self.bar_open = false;
        self.mode = Mode::Text;
        self.selection = Some(Selection::Chars { anchor });
        self.cursor = offset;
    }

    /// The wheel: scroll the part under the pointer. The focus and the selection stay.
    fn wheel(&mut self, (column, row): (u16, u16), forward: bool) {
        let hit = self.hits.at(column, row).copied();
        if let Some(editor) = self.editor.as_mut() {
            // The note box takes the wheel over itself. Elsewhere the wheel does nothing.
            if matches!(hit, Some(Hit::Editor | Hit::EditorSave | Hit::EditorCancel)) {
                editor.scroll_rows(forward, super::editor::WHEEL_ROWS);
            }
            return;
        }
        let boxed = self.boxed.is_some() || matches!(self.screen, Screen::Picker(_));
        let part = match hit {
            Some(Hit::ListRow(_) | Hit::BoxInside) if boxed => Part::Box,
            _ if boxed => return,
            Some(Hit::TreeRow(_) | Hit::TreePane) => Part::Tree,
            Some(Hit::ListRow(_) | Hit::NotesPane) => Part::Notes,
            Some(Hit::BoxInside) if self.focus == Focus::Tree => Part::Tree,
            Some(Hit::BoxInside) if self.focus == Focus::Rail => Part::Notes,
            Some(
                Hit::Block(_)
                | Hit::DocCell { .. }
                | Hit::Bar
                | Hit::Action(_)
                | Hit::Card(_)
                | Hit::RailPane
                | Hit::CardEdit(_)
                | Hit::CardRemove(_)
                | Hit::EditorSave
                | Hit::EditorCancel,
            ) => Part::Doc,
            _ => return,
        };
        // An item of the annotation list has two lines.
        let step = |top: usize, lines: usize| if forward { top + lines } else { top.saturating_sub(lines) };
        match part {
            Part::Doc => {
                self.wheel_doc(forward, WHEEL_LINES);
                self.scroll.doc = true;
            }
            // Several steps can come before the next frame: each one starts from the scroll so far.
            Part::Tree => {
                self.scroll.tree = Some(step(self.scroll.tree.unwrap_or(self.tops.tree), WHEEL_LINES));
            }
            Part::Notes => {
                let top = self.scroll.notes.unwrap_or(self.tops.notes);
                self.scroll.notes = Some(step(top, (WHEEL_LINES / 2).max(1)));
            }
            Part::Box => {
                self.scroll.boxed = Some(step(self.scroll.boxed.unwrap_or(self.tops.boxed), WHEEL_LINES));
            }
        }
    }
}
