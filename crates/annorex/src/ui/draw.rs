//! Drawing of the viewer with ratatui.

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::app::{App, Focus, Screen, match_ranges};
use crate::doc::{DocSpan, SpanStyle};
use crate::layout::{Layout, compute};
use crate::replies::Candidate;
use crate::wrap::{VisualLine, fit, gutter_width};

/// The key hints of the viewer, most important first.
const VIEWER_KEYS: &str = "j/k scroll  / find  n/N next  p replies  r reload  z zoom  q quit";
const PANEL_KEYS: &str = "j/k move  enter show  esc back  p close  q quit";
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
            draw_header(frame, header, app, "last reply", None);
            let body = Rect::new(area.x, header.bottom(), area.width, footer.y - header.bottom());
            let paragraph = Paragraph::new(format!("Annorex: {message}")).wrap(Wrap { trim: true });
            frame.render_widget(paragraph, body);
            draw_footer(frame, footer, ERROR_KEYS, None);
        }
        Screen::Picker(candidates) => {
            draw_header(frame, header, app, "choose the session", None);
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

fn draw_header(frame: &mut Frame, area: Rect, app: &App, view: &str, right: Option<String>) {
    let left = format!("Annorex · {view} · {}", app.state.header_text());
    let width = usize::from(area.width);
    let text = match right {
        Some(right) if left.chars().count() + 1 + right.chars().count() <= width => {
            let gap = width - left.chars().count() - right.chars().count();
            format!("{left}{}{right}", " ".repeat(gap))
        }
        _ => fit(&left, area.width),
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
    draw_header(frame, header, app, "last reply", Some(format!("reply {}/{count}", selected + 1)));
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
        draw_reply_list(frame, app, panel, borders);
    }
    let message = match &app.find_input {
        Some(input) => Some(format!("/{input}")),
        None => app.status.clone(),
    };
    let keys = if app.focus == Focus::Panel { PANEL_KEYS } else { VIEWER_KEYS };
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
    let lines: Vec<Line<'static>> = rows
        .iter()
        .skip(top_row)
        .take(height)
        .map(|row| {
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
            spans.extend(row_spans(row, query.as_deref()));
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

/// The spans of a row, with the matches of the query marked.
fn row_spans(row: &VisualLine, query: Option<&str>) -> Vec<Span<'static>> {
    let matches = query.map(|query| match_ranges(&row.plain(), query)).unwrap_or_default();
    let highlight = Style::new().fg(Color::Black).bg(Color::Yellow);
    let mut out = Vec::new();
    let mut position = 0;
    for DocSpan { text, style } in &row.spans {
        let base = style_of(*style);
        let mut current = String::new();
        let mut current_marked = false;
        for c in text.chars() {
            let marked = matches.iter().any(|range| range.contains(&position));
            if marked != current_marked && !current.is_empty() {
                let style = if current_marked { highlight } else { base };
                out.push(Span::styled(std::mem::take(&mut current), style));
            }
            current_marked = marked;
            current.push(c);
            position += 1;
        }
        if !current.is_empty() {
            out.push(Span::styled(current, if current_marked { highlight } else { base }));
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
            let time = short_time(reply.at.as_deref());
            let text = format!("{marker} {:>2} {time} {}", index + 1, reply.first_line());
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

/// `MM-DD HH:MM` for an RFC 3339 time. Other text stays as it is.
fn short_time(at: Option<&str>) -> String {
    match at {
        Some(at) if at.len() >= 16 && at.as_bytes().get(10) == Some(&b'T') => {
            format!("{} {}", at.get(5..10).unwrap_or(""), at.get(11..16).unwrap_or(""))
        }
        Some(at) => at.to_owned(),
        None => String::new(),
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
    for (position, candidate) in candidates.iter().enumerate() {
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
