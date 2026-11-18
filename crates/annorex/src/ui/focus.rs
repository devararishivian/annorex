//! Block mode and text mode: the mode type and the moves from block to block.
//!
//! The moves work on the rows of the last frame. The view can start only at the first row of a
//! `DocLine`, because `App::top` is a `DocLine`.

use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::app::{App, Selection};
use crate::doc::Block;
use crate::keys::Action;
use crate::review::reanchor_range;
use crate::wrap::VisualLine;

/// How the keys move in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Block mode without a focus: no block is chosen yet. The view starts so. `j` and `k` choose a
    /// block on the screen, a click chooses one, and `esc` in block mode comes back here.
    Browse,
    /// One block has the focus: its index in `Doc::blocks`. The keys move from block to block.
    Block(usize),
    /// A free cursor and a character selection, as in a text editor.
    Text,
}

/// A move in block mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Move {
    /// The next block, or a scroll inside a tall block.
    Down,
    /// The previous block, or a scroll inside a tall block.
    Up,
    First,
    Last,
    /// Scroll by this number of rows, then focus the first block that starts in the view. The focus
    /// moves only in the direction of the scroll.
    Scroll {
        forward: bool,
        rows: usize,
    },
    /// Keep the focus. Only put the block in view.
    Show,
}

/// The rows of the document and the part of them on the screen.
pub(crate) struct View<'a> {
    pub(crate) rows: &'a [VisualLine],
    /// The index of the first row on the screen.
    pub(crate) top: usize,
    /// The number of rows on the screen.
    pub(crate) height: usize,
}

impl View<'_> {
    /// The first row of the `DocLine` `line`, or the number of rows.
    fn first_row(&self, line: usize) -> usize {
        // The rows are in `DocLine` order.
        self.rows.partition_point(|row| row.doc_line < line)
    }

    /// The rows of a block.
    pub(crate) fn block_rows(&self, block: &Block) -> Range<usize> {
        self.first_row(block.lines.start)..self.first_row(block.lines.end)
    }

    /// The first row of the `DocLine` that holds `row`.
    fn start_at_or_before(&self, row: usize) -> usize {
        let row = row.min(self.rows.len().saturating_sub(1));
        self.rows.get(row).map_or(0, |visual| self.first_row(visual.doc_line))
    }

    /// `row` when a `DocLine` starts there, else the first row of the next `DocLine`.
    fn start_at_or_after(&self, row: usize) -> usize {
        match self.rows.get(row) {
            Some(visual) if visual.first => row,
            Some(visual) => self.first_row(visual.doc_line + 1),
            None => self.rows.len(),
        }
    }

    /// The top row nearest to `top` that keeps the block in view. A block that fits is fully on the
    /// screen. A taller block fills the screen, from its first row to the screen that shows its end.
    pub(crate) fn top_for(&self, top: usize, block: &Block) -> usize {
        let rows = self.block_rows(block);
        if rows.is_empty() {
            return top;
        }
        let height = self.height.max(1);
        let end_top = rows.end.saturating_sub(height);
        let mut last = self.start_at_or_after(end_top);
        if last >= rows.end {
            // The last line of the block is taller than the screen.
            last = self.start_at_or_before(end_top);
        }
        let (low, high) = (rows.start.min(last), rows.start.max(last));
        top.clamp(low, high)
    }

    /// The last top row of the view: the first `DocLine` start at or after the row that leaves the
    /// screen full. When the last `DocLine` is taller than the screen, its start. This is the rule of
    /// [`View::top_for`].
    fn max_top(&self) -> usize {
        let end_top = self.rows.len().saturating_sub(self.height);
        let top = self.start_at_or_after(end_top);
        if top >= self.rows.len() { self.start_at_or_before(end_top) } else { top }
    }

    /// The top row after a scroll by `amount` rows. The view starts at the first row of a `DocLine`,
    /// and it does not go past the screen that shows the last row.
    fn scroll_top(&self, forward: bool, amount: usize) -> usize {
        let max_top = self.max_top();
        if forward {
            self.start_at_or_before((self.top + amount).min(max_top)).max(self.top)
        } else {
            self.start_at_or_before(self.top.saturating_sub(amount))
        }
    }

    /// The top row after a scroll of the mouse. Unlike [`View::scroll_top`], it always moves while
    /// it can: a `DocLine` taller than `amount` rows does not stop it.
    fn wheel_top(&self, forward: bool, amount: usize) -> usize {
        let next = self.scroll_top(forward, amount);
        let max_top = self.max_top();
        if forward && next == self.top && self.top < max_top {
            self.start_at_or_after(self.top + 1).min(max_top)
        } else {
            next
        }
    }

    /// The top row after a move when no block has the focus: the keys only scroll the view.
    fn browse_top(&self, how: Move) -> usize {
        let max_top = self.max_top();
        match how {
            Move::First => 0,
            Move::Last => max_top,
            Move::Scroll { forward, rows } => self.scroll_top(forward, rows),
            Move::Down | Move::Up | Move::Show => self.top,
        }
    }

    /// The block to choose when no block has the focus: the first block (or the last block) whose
    /// first row is on the screen. A block that holds the top row is the second choice.
    pub(crate) fn browse_pick(&self, blocks: &[Block], last: bool) -> Option<usize> {
        let bottom = self.top + self.height.max(1);
        let starts = |b: &Block| {
            let rows = self.block_rows(b);
            !rows.is_empty() && rows.start >= self.top && rows.start < bottom
        };
        let found = if last { blocks.iter().rposition(starts) } else { blocks.iter().position(starts) };
        found.or_else(|| blocks.iter().position(|b| self.block_rows(b).contains(&self.top)))
    }

    /// The focus and the top row after a move. `focus` is the block that has the focus now.
    pub(crate) fn apply(&self, blocks: &[Block], focus: usize, how: Move) -> (usize, usize) {
        let Some(last) = blocks.len().checked_sub(1) else { return (focus, self.top) };
        let focus = focus.min(last);
        let show =
            |index: usize, top: usize| (index, blocks.get(index).map_or(top, |b| self.top_for(top, b)));
        let half = (self.height / 2).max(1);
        let Some(block) = blocks.get(focus) else { return (focus, self.top) };
        let top = self.top_for(self.top, block);
        let rows = self.block_rows(block);
        let tall = rows.len() > self.height.max(1);
        match how {
            Move::Show => (focus, top),
            Move::First => show(0, top),
            Move::Last => show(last, top),
            Move::Down => {
                if tall && rows.end > top + self.height {
                    let next = self.start_at_or_before((top + half).min(rows.end - self.height));
                    if next > top {
                        return (focus, next);
                    }
                }
                if focus < last { show(focus + 1, top) } else { (focus, top) }
            }
            Move::Up => {
                if tall && rows.start < top {
                    let next = self.start_at_or_after(top.saturating_sub(half).max(rows.start));
                    if next < top {
                        return (focus, next);
                    }
                }
                if focus > 0 { show(focus - 1, top) } else { (focus, top) }
            }
            Move::Scroll { forward, rows: amount } => {
                let next = self.scroll_top(forward, amount);
                let bottom = next + self.height.max(1);
                let starts_in_view = blocks.iter().position(|b| {
                    let rows = self.block_rows(b);
                    !rows.is_empty() && rows.start >= next && rows.start < bottom
                });
                let holds_top = || blocks.iter().position(|b| self.block_rows(b).contains(&next));
                let index = starts_in_view.or_else(holds_top).unwrap_or(focus);
                // The focus never goes against the direction of the scroll. A view that cannot move
                // must not throw the focus back.
                let index = if forward { index.max(focus) } else { index.min(focus) };
                show(index, next)
            }
        }
    }
}

/// The keys and the focus of block mode.
impl App {
    /// Block mode or text mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The block with the focus in block mode, or the block of the cursor in text mode.
    pub fn focused_block(&self) -> Option<usize> {
        match self.mode {
            Mode::Block(index) => (index < self.doc.blocks.len()).then_some(index),
            Mode::Browse => None,
            Mode::Text => self.doc.block_near(self.cursor),
        }
    }

    /// True in block mode, with or without a focused block.
    pub(super) fn in_block_mode(&self) -> bool {
        matches!(self.mode, Mode::Block(_) | Mode::Browse)
    }

    /// The first (or the last) block whose first row is on the screen. `None` for an empty document.
    fn block_on_screen(&mut self, last: bool) -> Option<usize> {
        let width = if self.width == 0 { u16::MAX } else { self.width };
        let top = self.view_top_row();
        let rows = self.cache.get(&self.doc, width);
        View { rows, top, height: self.page.max(1) }.browse_pick(&self.doc.blocks, last)
    }

    /// The source range of the focused block in block mode.
    pub(super) fn focus_range(&self) -> Option<Range<usize>> {
        let Mode::Block(index) = self.mode else { return None };
        self.doc.blocks.get(index).map(|block| block.range.clone())
    }

    /// The first cursor stop of a block.
    pub(super) fn block_start(&self, index: usize) -> Option<usize> {
        let block = self.doc.blocks.get(index)?;
        let line = block.lines.start;
        self.doc
            .stops(line)
            .into_iter()
            .find(|&stop| stop >= block.range.start)
            .or_else(|| self.line_start(line))
    }

    /// Do a move of block mode on the rows of the last frame. The cursor goes to the focused block.
    pub(super) fn block_move(&mut self, how: Move) {
        let width = if self.width == 0 { u16::MAX } else { self.width };
        let rows = self.cache.get(&self.doc, width);
        let top = rows.iter().position(|row| row.doc_line >= self.top).unwrap_or(0);
        let view = View { rows, top, height: self.page.max(1) };
        let focus = match self.mode {
            Mode::Block(focus) => focus,
            Mode::Browse => {
                // No focus: `j` and `k` choose a block on the screen. The other moves only scroll.
                let chosen = match how {
                    Move::Down => view.browse_pick(&self.doc.blocks, false),
                    Move::Up => view.browse_pick(&self.doc.blocks, true),
                    _ => None,
                };
                if let Some(index) = chosen {
                    self.mode = Mode::Block(index);
                    self.block_move(Move::Show);
                } else {
                    let next = view.browse_top(how);
                    self.top = rows.get(next).map_or(0, |row| row.doc_line);
                }
                return;
            }
            Mode::Text => return,
        };
        let (focus, top) = view.apply(&self.doc.blocks, focus, how);
        self.top = rows.get(top).map_or(0, |row| row.doc_line);
        self.mode = Mode::Block(focus);
        if let Some(start) = self.block_start(focus) {
            self.cursor = start;
        }
    }

    /// The index of the first visual row of the view.
    pub(super) fn view_top_row(&mut self) -> usize {
        let width = if self.width == 0 { u16::MAX } else { self.width };
        let top = self.top;
        self.cache.get(&self.doc, width).iter().position(|row| row.doc_line >= top).unwrap_or(0)
    }

    /// Scroll the view by `rows` rows for the mouse. The focus, the cursor, and the selection stay.
    pub(super) fn wheel_doc(&mut self, forward: bool, rows: usize) {
        let width = if self.width == 0 { u16::MAX } else { self.width };
        let top = self.view_top_row();
        let lines = self.cache.get(&self.doc, width);
        let next = View { rows: lines, top, height: self.page.max(1) }.wheel_top(forward, rows);
        self.top = lines.get(next).map_or(self.top, |row| row.doc_line);
    }

    /// In block mode, focus the block near `offset` and put it in view.
    pub(super) fn focus_near(&mut self, offset: usize) {
        if self.in_block_mode() {
            self.mode = Mode::Block(self.doc.block_near(offset).unwrap_or(0));
            self.block_move(Move::Show);
        }
    }

    /// After a new read in block mode: focus the old focused text again, else the block near its old offset.
    pub(super) fn follow_focus(&mut self, old: Option<Range<usize>>, changed: &[(String, String, String)]) {
        // No focus before the read: no focus after it.
        if self.mode == Mode::Browse {
            return;
        }
        let shown = self.shown_id.as_deref();
        let moved = old.as_ref().and_then(|old| {
            let (_, before, after) = changed.iter().find(|(id, ..)| Some(id.as_str()) == shown)?;
            reanchor_range(before, after, old)
        });
        self.focus_near(moved.or(old).map_or(0, |range| range.start));
    }

    /// Leave block mode with the cursor at the start of the focused block. False without a block.
    pub(super) fn enter_text(&mut self) -> bool {
        if self.mode == Mode::Browse {
            // The text mode starts at the first block on the screen.
            let Some(index) = self.block_on_screen(false) else { return false };
            self.mode = Mode::Block(index);
        }
        let Mode::Block(index) = self.mode else { return true };
        let Some(start) = self.block_start(index) else { return false };
        self.cursor = start;
        self.selection = None;
        self.mode = Mode::Text;
        true
    }

    /// A key of block mode. False when the key is not for block mode: the common keys handle it.
    pub(super) fn block_key(&mut self, key: KeyEvent) -> bool {
        let page = self.page.max(1);
        let half = (self.page / 2).max(1);
        let how = match key.code {
            KeyCode::PageDown => Move::Scroll { forward: true, rows: page },
            KeyCode::PageUp => Move::Scroll { forward: false, rows: page },
            KeyCode::Home => Move::First,
            KeyCode::End => Move::Last,
            KeyCode::Enter => {
                // `enter` opens the action strip for the whole focused block. Block mode stays.
                // Without a focus, it first focuses the first block on the screen.
                if self.mode == Mode::Browse
                    && let Some(index) = self.block_on_screen(false)
                {
                    self.mode = Mode::Block(index);
                    self.block_move(Move::Show);
                }
                self.bar_open = matches!(self.mode, Mode::Block(_));
                return true;
            }
            KeyCode::Esc => {
                // `esc` removes the focus. Without a focus, it does nothing.
                if matches!(self.mode, Mode::Block(_)) {
                    self.mode = Mode::Browse;
                }
                return true;
            }
            _ => match self.action_of(key) {
                Some(Action::Down) => Move::Down,
                Some(Action::Up) => Move::Up,
                Some(Action::Top) => Move::First,
                Some(Action::Bottom) => Move::Last,
                Some(Action::HalfPageDown) => Move::Scroll { forward: true, rows: half },
                Some(Action::HalfPageUp) => Move::Scroll { forward: false, rows: half },
                // Enter text mode at the start of the block, then the common keys do the move.
                Some(
                    Action::Left
                    | Action::Right
                    | Action::WordNext
                    | Action::WordPrev
                    | Action::LineStart
                    | Action::LineEnd,
                ) => return !self.enter_text(),
                Some(Action::Select) => {
                    if self.enter_text() {
                        self.selection = Some(Selection::Chars { anchor: self.cursor });
                    }
                    return true;
                }
                Some(Action::SelectLines) => {
                    if self.mode == Mode::Browse
                        && let Some(index) = self.block_on_screen(false)
                    {
                        self.mode = Mode::Block(index);
                    }
                    let last = self
                        .focused_block()
                        .and_then(|index| self.doc.blocks.get(index))
                        .map(|b| b.lines.end);
                    if self.enter_text() {
                        self.selection = Some(Selection::Lines { anchor: self.cursor });
                        let end = last.and_then(|end| self.line_start(end.saturating_sub(1)));
                        self.cursor = end.unwrap_or(self.cursor);
                    }
                    return true;
                }
                _ => return false,
            },
        };
        self.block_move(how);
        true
    }

    /// `esc` in text mode: clear the selection, else go back to block mode on the block of the cursor.
    pub(super) fn escape(&mut self) {
        if self.selection.take().is_none() {
            self.mode = Mode::Block(self.doc.block_near(self.cursor).unwrap_or(0));
            self.block_move(Move::Show);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing, reason = "tests assert by panicking")]

    use super::*;

    /// Rows for `DocLines` with the given heights.
    fn rows_of(heights: &[usize]) -> Vec<VisualLine> {
        heights
            .iter()
            .enumerate()
            .flat_map(|(line, &height)| {
                (0..height).map(move |n| VisualLine { doc_line: line, spans: Vec::new(), first: n == 0 })
            })
            .collect()
    }

    #[test]
    fn max_top_is_the_next_doc_line_start() {
        // 1 + 1 + 6 + 1 = 9 rows, height 4: row 5 is inside the tall line (rows 2..8).
        let rows = rows_of(&[1, 1, 6, 1]);
        let view = View { rows: &rows, top: 0, height: 4 };
        assert_eq!(view.max_top(), 8, "the start of the next DocLine");
    }

    #[test]
    fn max_top_of_a_last_line_taller_than_the_screen_is_its_start() {
        let rows = rows_of(&[1, 1, 8]);
        let view = View { rows: &rows, top: 0, height: 4 };
        assert_eq!(view.max_top(), 2);
        let at_end = View { rows: &rows, top: 2, height: 4 };
        assert_eq!(at_end.wheel_top(true, 3), 2, "no loop");
        assert_eq!(at_end.scroll_top(true, 3), 2);
    }

    #[test]
    fn max_top_after_a_tall_row_that_fits_on_the_screen() {
        // 2 + 3 + 2 = 7 rows, height 4: row 3 is inside the middle line (rows 2..5).
        let rows = rows_of(&[2, 3, 2]);
        let view = View { rows: &rows, top: 0, height: 4 };
        assert_eq!(view.max_top(), 5, "the old rule gave 2");
    }
}
