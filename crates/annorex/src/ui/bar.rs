//! The action strip: a short row with an item for each note type. It floats beside a finished
//! selection: one row above its first row, else one row below its last row.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Clear;
use unicode_width::UnicodeWidthChar;

use super::app::{App, Focus};
use super::cards::{icon, kind_color};
use super::hits::Hit;
use crate::keys::Action;
use crate::review::Kind;

/// The items of the strip, in order: the action of the key, the note type, the word.
const ITEMS: [(Action, Kind, &str); 4] = [
    (Action::LooksGood, Kind::LooksGood, "looks good"),
    (Action::Comment, Kind::Comment, "comment"),
    (Action::Delete, Kind::Delete, "remove"),
    (Action::Replace, Kind::Replace, "replace"),
];

/// The color of the strip.
const STRIP: Color = Color::Indexed(238);

/// The half blocks that end the strip on the left and on the right.
const LEFT_CAP: &str = "\u{2590}";
const RIGHT_CAP: &str = "\u{258c}";

/// The cells between two items.
const GAP: u16 = 2;

/// The first and the last row on the screen of a source range, as offsets from the top of the
/// document area. A range that is a whole block gives the rows of the block, its blank rows too.
/// Another range gives the rows that show a character of it. `None` when no row of it is on the screen.
pub(super) fn shown_rows(app: &mut App, range: &Range<usize>) -> Option<(usize, usize)> {
    shown_place(app, range).map(|(first, last, _)| (first, last))
}

/// The rows of [`shown_rows`] and the column where the range starts on its first row, as an offset
/// from the text column. The column is 0 for a whole block, for a range that starts on an earlier
/// row, and for a range that starts after only a prefix without source, such as a list mark.
fn shown_place(app: &mut App, range: &Range<usize>) -> Option<(usize, usize, u16)> {
    let lines = app.doc.blocks.iter().find(|block| block.range == *range).map(|block| block.lines.clone());
    let (top, height) = (app.top_row, app.page);
    let rows = app.cache.get(&app.doc, app.width);
    let mut shown = rows.iter().skip(top).take(height).enumerate().filter(|(_, row)| match &lines {
        Some(lines) => lines.contains(&row.doc_line),
        None => row.offsets().into_iter().flatten().any(|offset| range.contains(&offset)),
    });
    let (first, first_row) = shown.next()?;
    let last = shown.next_back().map_or(first, |(offset, _)| offset);
    let column =
        if lines.is_some() { 0 } else { start_column(&first_row.plain(), &first_row.offsets(), range) };
    Some((first, last, column))
}

/// The cells before the first character of `range` on a row. 0 when no character with a source
/// comes before it.
fn start_column(plain: &str, offsets: &[Option<usize>], range: &Range<usize>) -> u16 {
    let mut width = 0usize;
    let mut sourced = false;
    for (c, offset) in plain.chars().zip(offsets) {
        if offset.is_some_and(|offset| range.contains(&offset)) {
            return if sourced { u16::try_from(width).unwrap_or(u16::MAX) } else { 0 };
        }
        sourced |= offset.is_some();
        width += c.width().unwrap_or(0);
    }
    0
}

/// The place of a strip of `width` cells in `area`: one row above the row `first` at the column
/// `col`, else one row below the row `last`. `first`, `last`, and `col` are offsets from the top and
/// the left of `area`. The strip moves left to fit in `area`, and it is never wider than `area`.
/// `None` when neither row is in `area`.
pub(super) fn strip_rect(area: Rect, width: u16, first: usize, last: usize, col: u16) -> Option<Rect> {
    let row = match first.checked_sub(1) {
        Some(above) => above,
        None if last + 1 < usize::from(area.height) => last + 1,
        None => return None,
    };
    let row = u16::try_from(row).ok().filter(|&row| row < area.height)?;
    let width = width.min(area.width);
    let x = area.x.saturating_add(col).min(area.right() - width);
    Some(Rect::new(x, area.y + row, width, 1))
}

/// True when the strip does not show: no finished selection, an editor, the numbered pick, the
/// help screen, a question, or the find input is open, another part has the focus (the tree, the
/// rail, a list, or a box), or the document is empty.
fn hidden(app: &App) -> bool {
    !app.bar_open
        || app.editor.is_some()
        || app.pick.is_some()
        || app.help_open
        || app.confirm.is_some()
        || app.find_input.is_some()
        || app.focus != Focus::Doc
        || app.shown_id.is_none()
        || app.doc.blocks.is_empty()
}

/// Draw the action strip beside the range of a note after a finished selection: the selection, or
/// the cursor line after `enter` on a block. `area` is the text area of the document: it starts at
/// the text column. The items that do not fit are left out from the end.
pub(super) fn draw_action_bar(frame: &mut Frame, area: Rect, app: &mut App) {
    if hidden(app) || area.width == 0 || area.height == 0 {
        return;
    }
    let Some(range) = app.action_range().filter(|range| !range.is_empty()) else { return };
    let Some((first, last, col)) = shown_place(app, &range) else { return };
    let item_style = |kind: Kind| Style::new().fg(kind_color(kind)).bg(STRIP).add_modifier(Modifier::BOLD);
    let items: Vec<(Kind, Line)> = ITEMS
        .into_iter()
        .map(|(action, kind, word)| {
            let text = format!("{} {} {word}", app.config.keys.label(action), icon(kind));
            (kind, Line::from(Span::styled(text, item_style(kind))))
        })
        .collect();
    let widths: Vec<u16> =
        items.iter().map(|(_, line)| u16::try_from(line.width()).unwrap_or(u16::MAX)).collect();
    // The caps and the space inside each cap.
    let frame_cells = 4u16;
    let mut count = items.len();
    let total = |count: usize| -> u32 {
        let gaps = u32::try_from(count.saturating_sub(1)).unwrap_or(0) * u32::from(GAP);
        widths.iter().take(count).map(|&w| u32::from(w)).sum::<u32>() + gaps + u32::from(frame_cells)
    };
    while count > 0 && total(count) > u32::from(area.width) {
        count -= 1;
    }
    if count == 0 {
        return;
    }
    let Ok(width) = u16::try_from(total(count)) else { return };
    let Some(rect) = strip_rect(area, width, first, last, col) else { return };
    frame.render_widget(Clear, rect);
    let buffer = frame.buffer_mut();
    buffer.set_style(Rect { x: rect.x + 1, width: rect.width - 2, ..rect }, Style::new().bg(STRIP));
    let cap = Style::new().fg(STRIP);
    buffer.set_string(rect.x, rect.y, LEFT_CAP, cap);
    buffer.set_string(rect.right() - 1, rect.y, RIGHT_CAP, cap);
    // The whole strip belongs to the bar: a click between two items must not reach the text below.
    app.hits.push(rect, Hit::Bar);
    let mut x = rect.x + 2;
    for ((kind, line), width) in items.iter().zip(widths).take(count) {
        frame.buffer_mut().set_line(x, rect.y, line, width);
        app.hits.push(Rect::new(x, rect.y, width, 1), Hit::Action(*kind));
        x += width + GAP;
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use ratatui::layout::Rect;

    use super::strip_rect;

    #[test]
    fn strip_goes_above_the_first_row_at_the_column() {
        let area = Rect::new(4, 2, 60, 20);
        assert_eq!(strip_rect(area, 30, 5, 7, 3), Some(Rect::new(7, 6, 30, 1)));
    }

    #[test]
    fn strip_goes_below_the_last_row_at_the_top() {
        let area = Rect::new(4, 2, 60, 20);
        assert_eq!(strip_rect(area, 30, 0, 3, 3), Some(Rect::new(7, 6, 30, 1)));
        // No row above and no row below: no strip.
        assert_eq!(strip_rect(area, 30, 0, 19, 3), None);
    }

    #[test]
    fn strip_moves_left_at_right_edge() {
        let area = Rect::new(4, 2, 60, 20);
        for col in [40, 55, 59] {
            let rect = strip_rect(area, 30, 5, 5, col).expect("strip");
            assert!(rect.x + rect.width <= area.right(), "col {col}: {rect:?}");
            assert_eq!(rect.right(), area.right(), "col {col}: it moves left only as far as it must");
        }
        // A strip wider than the area starts at the left of the area and is cut to its width.
        assert_eq!(strip_rect(area, 80, 5, 5, 10), Some(Rect::new(4, 6, 60, 1)));
    }
}
