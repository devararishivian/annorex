//! The notes beside the document: margin cards in a rail on the right at wide widths, a colored
//! marker at narrower widths, and the numbered pick when several notes share one place.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, Focus};
use super::bar::shown_rows;
use super::draw::editor_top;
use super::hits::Hit;
use crate::keys::Action;
use crate::review::{Annotation, Kind};
use crate::wrap::fit;

/// The width of the marker margin on the right where the window has no card rail: a blank and the marker.
const MARKER_WIDTH: u16 = 2;
/// The narrowest document area that keeps the marker margin.
const MARKERS_FROM_WIDTH: u16 = 21;
/// The most rows of note text on one card.
const MAX_BODY_ROWS: usize = 6;
/// The most notes in the numbered pick: the keys `1` to `9`.
pub(super) const MAX_PICK: usize = 9;

/// What shows at the right of the document text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    /// The card rail. Its first column stays blank between the document and the cards.
    Rail(Rect),
    /// The marker margin.
    Markers(Rect),
    /// The area is too narrow for either.
    Nothing,
}

/// Where the window has no card rail: split the document area into the area of the text and the
/// marker margin at its right.
pub(super) fn split_markers(area: Rect) -> (Rect, Side) {
    if area.width >= MARKERS_FROM_WIDTH {
        let doc = Rect::new(area.x, area.y, area.width - MARKER_WIDTH, area.height);
        (doc, Side::Markers(Rect::new(doc.right(), area.y, MARKER_WIDTH, area.height)))
    } else {
        (area, Side::Nothing)
    }
}

/// The color of a note type.
pub(super) fn kind_color(kind: Kind) -> Color {
    match kind {
        Kind::LooksGood => Color::Green,
        Kind::Comment => Color::Yellow,
        Kind::Delete => Color::Red,
        Kind::Replace => Color::Magenta,
    }
}

/// The style of the text under a note of this type.
pub(super) fn range_style(kind: Kind) -> Style {
    match kind {
        Kind::LooksGood => Style::new().fg(Color::Black).bg(Color::Green),
        Kind::Comment => Style::new().fg(Color::Yellow).add_modifier(Modifier::UNDERLINED),
        Kind::Delete => Style::new().fg(Color::Red).add_modifier(Modifier::CROSSED_OUT),
        Kind::Replace => Style::new().fg(Color::Magenta).add_modifier(Modifier::CROSSED_OUT),
    }
}

/// Which type shows where notes of two types cover the same text: the higher rank.
pub(super) fn rank(kind: Kind) -> u8 {
    match kind {
        Kind::LooksGood => 0,
        Kind::Comment => 1,
        Kind::Replace => 2,
        Kind::Delete => 3,
    }
}

fn icon(kind: Kind) -> &'static str {
    match kind {
        Kind::LooksGood => "✓",
        Kind::Comment => "💬",
        Kind::Delete => "✕",
        Kind::Replace => "⇄",
    }
}

fn word(kind: Kind) -> &'static str {
    match kind {
        Kind::LooksGood => "looks good",
        Kind::Comment => "comment",
        Kind::Delete => "remove",
        Kind::Replace => "replace",
    }
}

/// The text of a note on its card: the replacement of a replace, else the note text.
fn body_of(annotation: &Annotation) -> &str {
    match annotation.kind {
        Kind::Replace => annotation.replacement.as_deref().unwrap_or_default(),
        _ => annotation.note.as_str(),
    }
}

/// Stack the cards in the rail. Each note gives its id, the row that it wants as an offset from
/// the top of the rail, and its height. The cards keep their order. A card that overlaps the card
/// above it goes down. A card that does not fit above the end of the rail is left out, and the
/// cards after it too.
pub(super) fn layout_cards(notes: &[(u64, u16, u16)], rail: Rect) -> Vec<(u64, Rect)> {
    let mut out = Vec::new();
    let mut free = rail.y;
    for &(id, wanted, height) in notes {
        let y = rail.y.saturating_add(wanted).max(free);
        if y.saturating_add(height) > rail.bottom() {
            break;
        }
        out.push((id, Rect::new(rail.x, y, rail.width, height)));
        free = y + height;
    }
    out
}

/// Wrap `text` to `width` columns at the spaces. A word longer than the width breaks anywhere.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in text.trim_end().split('\n') {
        let mut current = String::new();
        let mut used = 0;
        for word in line.split(' ') {
            let gap = usize::from(!current.is_empty());
            if used + gap + word.width() <= width {
                if gap == 1 {
                    current.push(' ');
                }
                current.push_str(word);
                used += gap + word.width();
                continue;
            }
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
                used = 0;
            }
            for c in word.chars() {
                let size = c.width().unwrap_or(0);
                if used + size > width && !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                    used = 0;
                }
                current.push(c);
                used += size;
            }
        }
        out.push(current);
    }
    out
}

/// A note on the screen: the note and the first row of its range, as an offset from the top of
/// the document area.
struct Shown {
    annotation: Annotation,
    row: u16,
    in_focus: bool,
}

/// The notes whose range has a row on the screen, in document order. Detached notes are left out.
/// A note has the focus when the rail has the focus and its card is the focused one, else when it is
/// at the focus of the document: on the focused block, or at the cursor in text mode.
fn shown_notes(app: &mut App) -> Vec<Shown> {
    let notes: Vec<Annotation> = app
        .review()
        .map(|review| review.in_order().into_iter().filter(|a| !a.detached).cloned().collect())
        .unwrap_or_default();
    let focused: Vec<u64> = if app.focus == Focus::Rail {
        app.listed_annotation().into_iter().collect()
    } else {
        app.notes_here()
    };
    notes
        .into_iter()
        .filter_map(|annotation| {
            let (first, _) = shown_rows(app, &annotation.range)?;
            let row = u16::try_from(first).ok()?;
            let in_focus = focused.contains(&annotation.id);
            Some(Shown { annotation, row, in_focus })
        })
        .collect()
}

/// Draw the cards or the markers of the notes on the screen. Call it after the document.
pub(super) fn draw_side(frame: &mut Frame, app: &mut App, side: Side) {
    match side {
        Side::Rail(rail) => draw_cards(frame, app, rail),
        Side::Markers(column) => draw_markers(frame, app, column),
        Side::Nothing => {}
    }
}

/// One `●` in the note color in the marker margin, on the first row of each note. Where notes
/// share a row, the first note gives the color.
fn draw_markers(frame: &mut Frame, app: &mut App, column: Rect) {
    if column.width < MARKER_WIDTH {
        return;
    }
    let mut used = Vec::new();
    for shown in shown_notes(app) {
        if shown.row >= column.height || used.contains(&shown.row) {
            continue;
        }
        used.push(shown.row);
        let style = Style::new().fg(kind_color(shown.annotation.kind));
        frame.buffer_mut().set_string(column.x + 1, column.y + shown.row, "●", style);
    }
}

/// The cards in the rail, each at the first row of its note when it can. A card has a border in
/// the note color, the icon and the word of the type in its title, and the note text inside. The
/// cards of the notes on the focused block are bold and have `edit` and `remove` buttons.
fn draw_cards(frame: &mut Frame, app: &mut App, rail: Rect) {
    if rail.width < 8 || rail.height < 3 {
        return;
    }
    let cards = Rect::new(rail.x + 1, rail.y, rail.width - 1, rail.height);
    let text_width = usize::from(cards.width.saturating_sub(4));
    let shown = shown_notes(app);
    let bodies: Vec<Vec<String>> = shown
        .iter()
        .map(|shown| {
            let mut rows = wrap_text(body_of(&shown.annotation), text_width);
            if rows.len() > MAX_BODY_ROWS {
                rows.truncate(MAX_BODY_ROWS);
                if let Some(last) = rows.last_mut() {
                    *last = fit(&format!("{last}…"), u16::try_from(text_width).unwrap_or(u16::MAX));
                }
            }
            rows
        })
        .collect();
    let wanted: Vec<(u64, u16, u16)> = shown
        .iter()
        .zip(&bodies)
        .map(|(shown, body)| (shown.annotation.id, shown.row, u16::try_from(body.len() + 2).unwrap_or(3)))
        .collect();
    let edit = app.config.keys.label(Action::Edit);
    let remove = app.config.keys.label(Action::Remove);
    for ((id, rect), (shown, body)) in layout_cards(&wanted, cards).into_iter().zip(shown.iter().zip(&bodies))
    {
        let annotation = &shown.annotation;
        let mut style = Style::new().fg(kind_color(annotation.kind));
        if shown.in_focus {
            style = style.add_modifier(Modifier::BOLD);
        }
        let label = annotation.label.as_ref().map_or_else(String::new, |label| format!("· {label} "));
        let title = format!(" {} {} {label}", icon(annotation.kind), word(annotation.kind));
        let block = Block::new().borders(Borders::ALL).border_style(style).title(Span::styled(title, style));
        let lines: Vec<Line<'static>> = body.iter().map(|row| Line::raw(format!(" {row}"))).collect();
        frame.render_widget(Clear, rect);
        frame.render_widget(Paragraph::new(lines).block(block), rect);
        app.hits.push(rect, Hit::Card(id));
        if shown.in_focus {
            draw_card_buttons(frame, app, rect, id, (&edit, &remove));
        }
    }
}

/// The ` KEY edit ` and ` KEY remove ` buttons on the bottom border of a card, at its right end.
fn draw_card_buttons(frame: &mut Frame, app: &mut App, rect: Rect, id: u64, keys: (&str, &str)) {
    let edit = format!(" {} edit ", keys.0);
    let remove = format!(" {} remove ", keys.1);
    let width = |text: &str| u16::try_from(text.width()).unwrap_or(u16::MAX);
    let (edit_width, remove_width) = (width(&edit), width(&remove));
    if rect.width < edit_width.saturating_add(remove_width).saturating_add(4) {
        return;
    }
    let y = rect.bottom() - 1;
    let remove_x = rect.right() - 2 - remove_width;
    let edit_x = remove_x - 1 - edit_width;
    let buffer = frame.buffer_mut();
    buffer.set_string(edit_x, y, &edit, Style::new());
    buffer.set_string(remove_x, y, &remove, Style::new());
    app.hits.push(Rect::new(edit_x, y, edit_width, 1), Hit::CardEdit(id));
    app.hits.push(Rect::new(remove_x, y, remove_width, 1), Hit::CardRemove(id));
}

/// The numbered pick: a small box at the place of the note with one row for each note, `1` to `9`.
/// `area` is the text area of the document.
pub(super) fn draw_pick(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(ids) = app.pick.clone() else { return };
    let Some(review) = app.review() else { return };
    let rows: Vec<(u64, String)> = ids
        .iter()
        .enumerate()
        .filter_map(|(index, id)| {
            let annotation = review.annotations.iter().find(|a| a.id == *id)?;
            let text = body_of(annotation).lines().next().unwrap_or_default();
            let text = if text.trim().is_empty() { word(annotation.kind) } else { text };
            Some((*id, format!("{} {} {text}", index + 1, icon(annotation.kind))))
        })
        .collect();
    let Ok(height) = u16::try_from(rows.len() + 2) else { return };
    if rows.is_empty() || area.width < 8 || area.height < height {
        return;
    }
    let longest = rows.iter().map(|(_, text)| text.width()).max().unwrap_or(0);
    let width = u16::try_from(longest + 2).unwrap_or(u16::MAX).clamp(14, area.width);
    let anchor = app.action_range().and_then(|range| shown_rows(app, &range));
    let rect = Rect::new(area.x, editor_top(area, height, anchor), width, height);
    let lines: Vec<Line<'static>> =
        rows.iter().map(|(_, text)| Line::raw(fit(text, width.saturating_sub(2)))).collect();
    let block = Block::new().borders(Borders::ALL).title(" pick ").title_bottom(" esc ");
    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines).block(block), rect);
    for (offset, (id, _)) in rows.iter().enumerate() {
        let y = rect.y + 1 + u16::try_from(offset).unwrap_or(0);
        app.hits.push(Rect::new(rect.x + 1, y, width.saturating_sub(2), 1), Hit::Pick(*id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_cards_stacks_without_overlap() {
        let rail = Rect::new(80, 0, 34, 20);
        let cards = layout_cards(&[(1, 2, 3), (2, 3, 3), (3, 3, 3)], rail);
        let ys: Vec<(u64, u16)> = cards.iter().map(|(id, rect)| (*id, rect.y)).collect();
        assert_eq!(ys, vec![(1, 2), (2, 5), (3, 8)]);
        assert!(cards.iter().all(|(_, rect)| rect.x >= rail.x && rect.right() <= rail.right()));
    }

    #[test]
    fn cards_below_the_rail_end_are_left_out() {
        let rail = Rect::new(80, 5, 34, 10);
        let cards = layout_cards(&[(1, 0, 4), (2, 1, 4), (3, 2, 4)], rail);
        let ys: Vec<(u64, u16)> = cards.iter().map(|(id, rect)| (*id, rect.y)).collect();
        assert_eq!(ys, vec![(1, 5), (2, 9)], "the third card starts below the rail end");
        assert_eq!(cards.get(1).map(|(_, rect)| rect.bottom()), Some(13));
    }

    #[test]
    fn wrap_text_breaks_at_spaces_and_long_words() {
        assert_eq!(wrap_text("one two three", 7), vec!["one two", "three"]);
        assert_eq!(wrap_text("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap_text("", 4), vec![""]);
        assert_eq!(wrap_text("a\nb", 4), vec!["a", "b"]);
    }

    #[test]
    fn the_marker_margin_needs_21_columns() {
        let (doc, side) = split_markers(Rect::new(0, 1, 119, 30));
        assert_eq!((doc.width, side), (117, Side::Markers(Rect::new(117, 1, 2, 30))));
        let (doc, side) = split_markers(Rect::new(0, 1, 20, 30));
        assert_eq!((doc.width, side), (20, Side::Nothing));
    }
}
