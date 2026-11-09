//! Drawing of the viewer with ratatui.

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use super::app::{App, Focus, PanelView, Screen, match_ranges};
use crate::doc::{DocSpan, SpanStyle};
use crate::layout::{Layout, compute};
use crate::replies::Candidate;
use crate::review::Kind;
use crate::wrap::{VisualLine, fit, gutter_width};

/// The key hints of the viewer, most important first.
const VIEWER_KEYS: &str = "j/k move  v select  c/d/s annotate  a list  p replies  / find  q quit";
const PANEL_KEYS: &str = "j/k move  enter show  esc back  p close  q quit";
const ANNOTATION_KEYS: &str = "j/k move  enter go  e edit  x delete  a close  esc back  q quit";
const EDITOR_KEYS: &str = "enter save  ctrl+j new line  tab label  esc cancel";
const PICKER_KEYS: &str = "j/k move  enter open  q quit";
const ERROR_KEYS: &str = "r try again  q quit";

/// Draw the whole viewer.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let layout = compute(area, app.panel_open);
    let Some((header, footer)) = header_footer(&layout) else {
        draw_too_small(frame, area);
        return;
    };
    match app.screen.clone() {
        Screen::Error(message) => {
            draw_header(frame, header, app, "last reply", &[]);
            let body = Rect::new(area.x, header.bottom(), area.width, footer.y - header.bottom());
            let paragraph = Paragraph::new(format!("Annorex: {message}")).wrap(Wrap { trim: true });
            frame.render_widget(paragraph, body);
            draw_footer(frame, footer, ERROR_KEYS, None);
        }
        Screen::Picker(candidates) => {
            draw_header(frame, header, app, "choose the session", &[]);
            let body = Rect::new(area.x, header.bottom(), area.width, footer.y - header.bottom());
            draw_picker(frame, body, &candidates, app.picker_index);
            draw_footer(frame, footer, PICKER_KEYS, None);
        }
        Screen::Viewer => draw_viewer(frame, app, layout, header, footer),
    }
}

fn header_footer(layout: &Layout) -> Option<(Rect, Rect)> {
    match *layout {
        Layout::TooSmall => None,
        Layout::Single { header, footer, .. }
        | Layout::Bottom { header, footer, .. }
        | Layout::Side { header, footer, .. } => Some((header, footer)),
    }
}

fn draw_too_small(frame: &mut Frame, area: Rect) {
    if area.height == 0 {
        return;
    }
    let row = Rect::new(area.x, area.y + area.height / 2, area.width, 1);
    let text = fit("pane too small", area.width);
    frame.render_widget(Paragraph::new(text).centered(), row);
}

/// The header. `rights` are the texts for the right side, the preferred one first. The first
/// that fits stays.
fn draw_header(frame: &mut Frame, area: Rect, app: &App, view: &str, rights: &[String]) {
    let left = format!("Annorex · {view} · {}", app.state.header_text());
    let width = usize::from(area.width);
    let text = match rights.iter().find(|right| left.width() + 1 + right.width() <= width) {
        Some(right) => {
            let gap = width - left.width() - right.width();
            format!("{left}{}{right}", " ".repeat(gap))
        }
        None => fit(&left, area.width),
    };
    frame.render_widget(Paragraph::new(text).style(Style::new().add_modifier(Modifier::BOLD)), area);
}

fn draw_footer(frame: &mut Frame, area: Rect, keys: &str, message: Option<String>) {
    let (text, style) = match message {
        Some(message) => (message, Style::new().add_modifier(Modifier::BOLD)),
        None => (keys.to_owned(), Style::new().add_modifier(Modifier::DIM)),
    };
    frame.render_widget(Paragraph::new(fit(&text, area.width)).style(style), area);
}

fn draw_viewer(frame: &mut Frame, app: &mut App, layout: Layout, header: Rect, footer: Rect) {
    let (selected, count) = app.reply_position();
    let position = format!("reply {}/{count}", selected + 1);
    let unsent = app.unsent_replies();
    let mut rights = Vec::new();
    if !unsent.is_empty() {
        let numbers: Vec<String> = unsent.iter().map(ToString::to_string).collect();
        rights.push(format!("{position} · unsent {}", numbers.join(",")));
    }
    rights.push(position);
    draw_header(frame, header, app, "last reply", &rights);
    let (doc_area, panel_area, overlay) = match layout {
        Layout::Single { doc, overlay, .. } => (doc, app.panel_open.then_some(overlay), true),
        Layout::Bottom { doc, panel, .. } | Layout::Side { doc, panel, .. } => {
            (doc, app.panel_open.then_some(panel), false)
        }
        Layout::TooSmall => return,
    };
    app.overlay = overlay;
    draw_doc(frame, app, doc_area);
    if let Some(panel) = panel_area {
        let borders = match layout {
            Layout::Side { .. } => Borders::LEFT,
            Layout::Bottom { .. } => Borders::TOP,
            _ => Borders::ALL,
        };
        if overlay {
            frame.render_widget(Clear, panel);
        }
        match app.panel_view {
            PanelView::Replies => draw_reply_list(frame, app, panel, borders),
            PanelView::Annotations => draw_annotation_list(frame, app, panel, borders),
        }
    }
    draw_editor(frame, app, doc_area);
    let prompt = app.confirm.map(|id| {
        let number = app.review().and_then(|review| review.in_order().iter().position(|a| a.id == id));
        format!("Delete annotation {}? y/n", number.map_or(0, |index| index + 1))
    });
    let message = prompt
        .or_else(|| app.find_input.as_ref().map(|input| format!("/{input}")))
        .or_else(|| app.status.clone())
        .or_else(|| app.save_error.as_ref().map(|error| format!("draft not saved: {error}")));
    let keys = if app.editor.is_some() {
        EDITOR_KEYS
    } else if app.focus == Focus::Panel && app.panel_view == PanelView::Annotations {
        ANNOTATION_KEYS
    } else if app.focus == Focus::Panel {
        PANEL_KEYS
    } else {
        VIEWER_KEYS
    };
    draw_footer(frame, footer, keys, message);
}

fn draw_doc(frame: &mut Frame, app: &mut App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let height = usize::from(area.height);
    app.page = height;
    let gutter = gutter_width(&app.doc).min(area.width.saturating_sub(1));
    let text_width = area.width - gutter;
    let cursor_line = app.cursor_line();
    let annotated: Vec<std::ops::Range<usize>> = app
        .review()
        .map(|review| review.annotations.iter().filter(|a| !a.detached).map(|a| a.range.clone()).collect())
        .unwrap_or_default();
    let selection = app.selection_range();
    let rows = app.cache.get(&app.doc, text_width);
    let first_row = |doc_line: usize| rows.iter().position(|row| row.doc_line >= doc_line);
    let mut top_row = first_row(app.top).unwrap_or(0);
    if let Some(cursor_row) = first_row(cursor_line) {
        if cursor_row < top_row {
            top_row = cursor_row;
        } else if cursor_row >= top_row + height {
            let wanted = cursor_row + 1 - height;
            let line = rows.get(wanted).map_or(0, |row| row.doc_line);
            top_row = first_row(line)
                .filter(|&row| row >= wanted)
                .or_else(|| first_row(line + 1))
                .unwrap_or(wanted);
        }
    }
    app.top = rows.get(top_row).map_or(0, |row| row.doc_line);
    let query = app.query.clone();
    // The cursor cell: the first row of the cursor line that shows the cursor stop.
    let cursor_cell = (app.focus == Focus::Doc)
        .then(|| {
            let start = first_row(cursor_line)?;
            rows.iter()
                .enumerate()
                .skip(start)
                .take_while(|(_, row)| row.doc_line == cursor_line)
                .find_map(|(index, row)| row.cell_of(app.cursor).map(|cell| (index, cell)))
        })
        .flatten();
    let lines: Vec<Line<'static>> = rows
        .iter()
        .skip(top_row)
        .take(height)
        .enumerate()
        .map(|(offset, row)| {
            let matches =
                query.as_deref().map(|query| row_matches(rows, top_row + offset, query)).unwrap_or_default();
            let number_style = if row.doc_line == cursor_line {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::new().add_modifier(Modifier::DIM)
            };
            let number = match app.doc.lines.get(row.doc_line) {
                Some(line) if row.first && !line.is_blank() => line.line.to_string(),
                _ => String::new(),
            };
            let number_width = usize::from(gutter.saturating_sub(1));
            let mut spans = vec![
                Span::styled(format!("{number:>number_width$}"), number_style),
                Span::raw(if gutter > 0 { " " } else { "" }),
            ];
            let cursor = cursor_cell.filter(|&(index, _)| index == top_row + offset).map(|(_, cell)| cell);
            let offsets = row.offsets();
            let flags = |ranges: &[std::ops::Range<usize>]| -> Vec<bool> {
                offsets
                    .iter()
                    .map(|at| at.is_some_and(|at| ranges.iter().any(|range| range.contains(&at))))
                    .collect()
            };
            let marks = Marks { underline: flags(&annotated), selected: flags(selection.as_slice()) };
            spans.extend(row_spans(row, &matches, cursor, &marks));
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn style_of(style: SpanStyle) -> Style {
    let mut out = Style::new();
    if let Some(level) = style.heading {
        out = out.fg(Color::Cyan).add_modifier(Modifier::BOLD);
        if level == 1 {
            out = out.add_modifier(Modifier::UNDERLINED);
        }
    }
    if style.strong {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.emphasis {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.code || style.code_block {
        out = out.fg(Color::Yellow);
    }
    if style.strike {
        out = out.add_modifier(Modifier::CROSSED_OUT);
    }
    if style.link {
        out = out.fg(Color::Blue).add_modifier(Modifier::UNDERLINED);
    }
    if style.quote || style.rule {
        out = out.add_modifier(Modifier::DIM);
    }
    out
}

/// The matches of `query` in row `index`, as character ranges of the row. A match can go over a
/// wrap: the rows of one `DocLine` are joined with the space that the wrap removed.
fn row_matches(rows: &[VisualLine], index: usize, query: &str) -> Vec<std::ops::Range<usize>> {
    let Some(row) = rows.get(index) else { return Vec::new() };
    let start = rows.partition_point(|other| other.doc_line < row.doc_line);
    let group: Vec<&VisualLine> =
        rows.iter().skip(start).take_while(|other| other.doc_line == row.doc_line).collect();
    let texts: Vec<String> = group.iter().map(|row| row.plain()).collect();
    let joined = texts.join(" ");
    let mut begin = 0;
    let mut mine = (0, 0);
    for (position, text) in texts.iter().enumerate() {
        let length = text.chars().count();
        if start + position == index {
            mine = (begin, begin + length);
        }
        begin += length + 1;
    }
    match_ranges(&joined, query)
        .into_iter()
        .filter_map(|range| {
            let (from, to) = (range.start.max(mine.0), range.end.min(mine.1));
            (from < to).then(|| from - mine.0..to - mine.0)
        })
        .collect()
}

/// Which characters of a row are annotated and which are selected.
struct Marks {
    underline: Vec<bool>,
    selected: Vec<bool>,
}

/// The spans of a row, with the matches marked, the cursor cell reversed, annotated text underlined,
/// and the selection reversed.
fn row_spans(
    row: &VisualLine,
    matches: &[std::ops::Range<usize>],
    cursor: Option<usize>,
    marks: &Marks,
) -> Vec<Span<'static>> {
    let highlight = Style::new().fg(Color::Black).bg(Color::Yellow);
    let mut out = Vec::new();
    let mut position = 0;
    for DocSpan { text, style, .. } in &row.spans {
        let base = style_of(*style);
        let style_at = |position: usize| {
            let mut style = if cursor == Some(position) {
                base.add_modifier(Modifier::REVERSED)
            } else if matches.iter().any(|range| range.contains(&position)) {
                highlight
            } else {
                base
            };
            if marks.selected.get(position) == Some(&true) {
                style = style.add_modifier(Modifier::REVERSED);
            }
            if marks.underline.get(position) == Some(&true) {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            style
        };
        let mut current = String::new();
        let mut current_style = base;
        for c in text.chars() {
            let style = style_at(position);
            if style != current_style && !current.is_empty() {
                out.push(Span::styled(std::mem::take(&mut current), current_style));
            }
            current_style = style;
            current.push(c);
            position += 1;
        }
        if !current.is_empty() {
            out.push(Span::styled(current, current_style));
        }
    }
    out
}

fn draw_reply_list(frame: &mut Frame, app: &App, area: Rect, borders: Borders) {
    let block = Block::new().borders(borders).title(" Replies ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let height = usize::from(inner.height);
    let offset = app.panel_index.saturating_sub(height.saturating_sub(1));
    let lines: Vec<Line<'static>> = app
        .replies
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, reply)| {
            let marker = if index == app.selected { "▸" } else { " " };
            let time = short_time(reply.at.as_deref(), &app.zone);
            let unsent = app.reviews.get(&reply.id).is_some_and(|review| !review.is_empty())
                && !app.sent.contains_key(&reply.id);
            let dot = if unsent { "●" } else { " " };
            let text = format!("{marker} {:>2}{dot}{time} {}", index + 1, reply.first_line());
            let style = if index == app.panel_index && app.focus == Focus::Panel {
                Style::new().add_modifier(Modifier::REVERSED)
            } else if index == app.selected {
                Style::new().add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            Line::styled(fit(&text, inner.width), style)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::Comment => "comment",
        Kind::Delete => "delete",
        Kind::Replace => "replace",
    }
}

fn draw_annotation_list(frame: &mut Frame, app: &App, area: Rect, borders: Borders) {
    let block = Block::new().borders(borders).title(" Annotations ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let items = app.review().map(crate::review::Review::in_order).unwrap_or_default();
    if items.is_empty() {
        frame.render_widget(Paragraph::new(fit("no annotations yet", inner.width)), inner);
        return;
    }
    let visible = usize::from(inner.height / 2).max(1);
    let offset = app.annotation_index.saturating_sub(visible - 1);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (index, annotation) in items.iter().enumerate().skip(offset).take(visible) {
        let bang = if annotation.detached { "!" } else { " " };
        let label = annotation.label.as_ref().map_or_else(String::new, |label| format!("  [{label}]"));
        let first = format!(
            "{bang}{:>2}  l.{}  {}{label}",
            index + 1,
            app.doc.line_of(annotation.range.start),
            kind_word(annotation.kind)
        );
        let detail = match annotation.kind {
            Kind::Replace => annotation.replacement.as_deref().unwrap_or_default(),
            _ if annotation.note.trim().is_empty() => annotation.quote.as_str(),
            _ => annotation.note.as_str(),
        };
        let second = format!("    {}", detail.lines().next().unwrap_or_default());
        let style = if index == app.annotation_index && app.focus == Focus::Panel {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        lines.push(Line::styled(fit(&first, inner.width), style));
        lines.push(Line::styled(fit(&second, inner.width), style.add_modifier(Modifier::DIM)));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The editor box at the bottom of the document area.
fn draw_editor(frame: &mut Frame, app: &App, area: Rect) {
    let Some(editor) = &app.editor else { return };
    if area.width < 4 || area.height < 5 {
        return;
    }
    let width = usize::from(area.width - 2);
    let quote = app.doc.source.get(editor.range.clone()).unwrap_or_default();
    let mut first = quote.lines().next().unwrap_or_default().to_owned();
    if quote.lines().count() > 1 {
        first.push('…');
    }
    let word = match editor.kind {
        Kind::Comment => "Comment on",
        Kind::Delete => "Remove",
        Kind::Replace => "Replace",
    };
    let label = editor.label.as_deref().unwrap_or("none");
    let mut lines = vec![
        Line::styled(
            fit(&format!("{word}: \"{first}\""), area.width - 2),
            Style::new().add_modifier(Modifier::DIM),
        ),
        Line::raw(fit(&format!("label: {label}"), area.width - 2)),
    ];
    let typed = format!("{}▏", editor.text);
    let text_rows: usize = typed.split('\n').map(|line| line.width().max(1).div_ceil(width)).sum();
    lines.extend(typed.split('\n').map(|line| Line::raw(line.to_owned())));
    let height = u16::try_from(text_rows.min(6) + 4).unwrap_or(u16::MAX).min(area.height);
    let rect = Rect::new(area.x, area.bottom() - height, area.width, height);
    let prefix = if app.editing_id.is_some() { "Edit " } else { "" };
    let title = format!(" {prefix}{} · l.{} ", kind_word(editor.kind), app.doc.line_of(editor.range.start));
    frame.render_widget(Clear, rect);
    let block = Block::new().borders(Borders::ALL).title(title);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), rect);
}

/// `MM-DD HH:MM` in `zone` for an RFC 3339 time. Other text stays as it is.
fn short_time(at: Option<&str>, zone: &jiff::tz::TimeZone) -> String {
    let Some(at) = at else { return String::new() };
    match at.parse::<jiff::Timestamp>() {
        Ok(time) => time.to_zoned(zone.clone()).strftime("%m-%d %H:%M").to_string(),
        Err(_) => at.to_owned(),
    }
}

fn draw_picker(frame: &mut Frame, area: Rect, candidates: &[Candidate], index: usize) {
    let now = SystemTime::now();
    let mut lines = vec![
        Line::raw(fit(
            "More than one session can belong to this agent. Choose one (newest first):",
            area.width,
        )),
        Line::raw(""),
    ];
    // The rows under the two lines of text. Scroll so that the selected row stays visible.
    let rows = usize::from(area.height).saturating_sub(lines.len()).max(1);
    let offset = index.saturating_sub(rows - 1);
    for (position, candidate) in candidates.iter().enumerate().skip(offset).take(rows) {
        let marker = if position == index { "▸" } else { " " };
        let text = format!("{marker} {} · {}", age(now, candidate.updated), candidate.last_line);
        let style =
            if position == index { Style::new().add_modifier(Modifier::REVERSED) } else { Style::new() };
        lines.push(Line::styled(fit(&text, area.width), style));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn age(now: SystemTime, then: SystemTime) -> String {
    let seconds = now.duration_since(then).map_or(0, |age| age.as_secs());
    match seconds {
        0..60 => "updated just now".to_owned(),
        60..3600 => format!("updated {} min ago", seconds / 60),
        3600..86_400 => format!("updated {} h ago", seconds / 3600),
        _ => format!("updated {} d ago", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use jiff::tz::{TimeZone, offset};

    use super::short_time;

    #[test]
    fn reply_time_is_local() {
        let zone = TimeZone::fixed(offset(7));
        assert_eq!(short_time(Some("2026-11-09T10:48:00Z"), &zone), "11-09 17:48");
        assert_eq!(short_time(Some("2026-11-09T10:30:30.000Z"), &TimeZone::UTC), "11-09 10:30");
        let west = TimeZone::fixed(offset(-5));
        assert_eq!(short_time(Some("2026-11-09T02:10:00+00:00"), &west), "11-08 21:10", "the date moves too");
    }

    #[test]
    fn other_time_text_stays_as_it_is() {
        let zone = TimeZone::fixed(offset(7));
        assert_eq!(short_time(Some("yesterday"), &zone), "yesterday");
        assert_eq!(short_time(None, &zone), "");
    }
}
