//! The action bar: one row with a chip for each note type, at the focused block or the selection.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Clear;

use super::app::{App, Focus};
use super::hits::Hit;
use crate::keys::Action;
use crate::review::Kind;

/// The buttons of the bar, in order: the action of the key, the note type, the word, the key color.
const BUTTONS: [(Action, Kind, &str, Color); 4] = [
    (Action::LooksGood, Kind::LooksGood, "looks good", Color::Green),
    (Action::Comment, Kind::Comment, "comment", Color::Yellow),
    (Action::Delete, Kind::Delete, "remove", Color::Red),
    (Action::Replace, Kind::Replace, "replace", Color::Magenta),
];

/// The first and the last row on the screen of a source range, as offsets from the top of the
/// document area. A range that is a whole block gives the rows of the block, its blank rows too.
/// Another range gives the rows that show a character of it. `None` when no row of it is on the screen.
pub(super) fn shown_rows(app: &mut App, range: &Range<usize>) -> Option<(usize, usize)> {
    let lines = app.doc.blocks.iter().find(|block| block.range == *range).map(|block| block.lines.clone());
    let (top, height) = (app.top_row, app.page);
    let rows = app.cache.get(&app.doc, app.width);
    let mut shown = rows.iter().skip(top).take(height).enumerate().filter(|(_, row)| match &lines {
        Some(lines) => lines.contains(&row.doc_line),
        None => row.offsets().into_iter().flatten().any(|offset| range.contains(&offset)),
    });
    let first = shown.next().map(|(offset, _)| offset)?;
    let last = shown.next_back().map_or(first, |(offset, _)| offset);
    Some((first, last))
}

/// True when the bar does not show: an editor, the numbered pick, the help screen, a question, or the find input is
/// open, another part has the focus (the tree, the rail, a list, or a box), or the document is empty.
fn hidden(app: &App) -> bool {
    app.editor.is_some()
        || app.pick.is_some()
        || app.help_open
        || app.confirm.is_some()
        || app.find_input.is_some()
        || app.focus != Focus::Doc
        || app.shown_id.is_none()
        || app.doc.blocks.is_empty()
}

/// Draw the action bar on the row below the range of a note: the focused block in block mode, the
/// selection or the cursor line in text mode. When that row is not in `area`, the bar goes on the
/// row above the range. When neither row is in `area`, no bar shows. `area` is the text area of the
/// document: it starts at the text column. `row_start` is the first column of the document row: the
/// bar row is blank from there, so no line number or focus mark of a hidden row shows. The buttons
/// that do not fit are left out from the end.
pub(super) fn draw_action_bar(frame: &mut Frame, area: Rect, row_start: u16, app: &mut App) {
    if hidden(app) || area.width == 0 || area.height == 0 {
        return;
    }
    let Some(range) = app.action_range().filter(|range| !range.is_empty()) else { return };
    let Some((first, last)) = shown_rows(app, &range) else { return };
    let row = if last + 1 < usize::from(area.height) {
        last + 1
    } else if let Some(above) = first.checked_sub(1) {
        above
    } else {
        return;
    };
    let Ok(row) = u16::try_from(row) else { return };
    let y = area.y + row;
    let whole = Rect::new(row_start, y, area.right().saturating_sub(row_start), 1);
    frame.render_widget(Clear, whole);
    // The whole row belongs to the bar: a click on its empty part must not reach the hidden row.
    app.hits.push(whole, Hit::Bar);
    let mut x = area.x;
    for (index, (action, kind, word, color)) in BUTTONS.into_iter().enumerate() {
        let key = app.config.keys.label(action);
        // A chip: bright black background, the key bold in the note color, the word plain. The gap
        // between two chips has no background.
        let chip = Style::new().bg(Color::DarkGray);
        let line = Line::from(vec![
            Span::styled(" ", chip),
            Span::styled(key, chip.fg(color).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {word} "), chip),
        ]);
        let Ok(width) = u16::try_from(line.width()) else { break };
        let start = if index == 0 { x } else { x + 1 };
        if start + width > area.right() {
            break;
        }
        frame.buffer_mut().set_line(start, y, &line, width);
        app.hits.push(Rect::new(start, y, width, 1), Hit::Action(kind));
        x = start + width;
    }
}
