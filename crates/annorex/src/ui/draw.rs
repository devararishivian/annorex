//! Drawing of the viewer with ratatui.

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use super::app::{App, Confirm, Focus, PanelView, ReplySource, Screen, match_ranges};
use crate::agent::Agent;
use crate::doc::{DocSpan, SpanStyle};
use crate::files::TREE_LIMIT;
use crate::keys::{Action, KeyMap};
use crate::layout::{Layout, compute};
use crate::replies::Candidate;
use crate::review::{Kind, Review};
use crate::safety::Safety;
use crate::wrap::{VisualLine, fit, gutter_width};

/// The key hints of the viewer, most important first. They show the keys that are bound now.
fn viewer_keys(keys: &KeyMap) -> String {
    let k = |action| keys.label(action);
    format!(
        "{}/{} move  {} select  {}/{}/{} annotate  {} list  {} replies  {} find  {} quit",
        k(Action::Down),
        k(Action::Up),
        k(Action::Select),
        k(Action::Comment),
        k(Action::Delete),
        k(Action::Replace),
        k(Action::Annotations),
        k(Action::Replies),
        k(Action::Find),
        k(Action::Quit)
    )
}

/// The key hints in the file viewer.
fn files_keys(keys: &KeyMap) -> String {
    let k = |action| keys.label(action);
    format!(
        "{}/{} move  {} select  {}/{}/{} annotate  {} files  {} tree  {} find file  {}/{} switch  {} quit",
        k(Action::Down),
        k(Action::Up),
        k(Action::Select),
        k(Action::Comment),
        k(Action::Delete),
        k(Action::Replace),
        k(Action::Replies),
        k(Action::Tree),
        k(Action::FindFile),
        k(Action::PrevFile),
        k(Action::NextFile),
        k(Action::Quit)
    )
}

fn tree_keys(keys: &KeyMap) -> String {
    let k = |action| keys.label(action);
    format!(
        "{}/{} move  enter open  {}/{} close/open  {} filter  esc close  {} quit",
        k(Action::Down),
        k(Action::Up),
        k(Action::Left),
        k(Action::Right),
        k(Action::Find),
        k(Action::Quit)
    )
}

const FINDER_KEYS: &str = "type to filter  enter open  ctrl+n/ctrl+p move  esc close";

/// A number with a comma between the thousands: `20,000`.
fn thousands(number: usize) -> String {
    let digits = number.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn panel_keys(keys: &KeyMap) -> String {
    let k = |action| keys.label(action);
    format!(
        "{}/{} move  enter show  esc back  {} close  {} quit",
        k(Action::Down),
        k(Action::Up),
        k(Action::Replies),
        k(Action::Quit)
    )
}

fn annotation_keys(keys: &KeyMap) -> String {
    let k = |action| keys.label(action);
    format!(
        "{}/{} move  enter go  {} edit  {} delete  {} close  esc back  {} quit",
        k(Action::Down),
        k(Action::Up),
        k(Action::Edit),
        k(Action::Remove),
        k(Action::Annotations),
        k(Action::Quit)
    )
}

fn picker_keys(keys: &KeyMap) -> String {
    format!(
        "{}/{} move  enter open  {} quit",
        keys.label(Action::Down),
        keys.label(Action::Up),
        keys.label(Action::Quit)
    )
}

fn error_keys(keys: &KeyMap) -> String {
    format!("{} try again  {} quit", keys.label(Action::Reload), keys.label(Action::Quit))
}

const EDITOR_KEYS: &str = "enter save  ctrl+j new line  tab label  esc cancel";

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
            draw_header(frame, header, app, title_of(app), &[]);
            let body = Rect::new(area.x, header.bottom(), area.width, footer.y - header.bottom());
            let paragraph = Paragraph::new(format!("Annorex: {message}")).wrap(Wrap { trim: true });
            frame.render_widget(paragraph, body);
            draw_footer(frame, footer, &error_keys(&app.config.keys), None);
        }
        Screen::Picker(candidates) => {
            draw_header(frame, header, app, "choose the session", &[]);
            let body = Rect::new(area.x, header.bottom(), area.width, footer.y - header.bottom());
            draw_picker(frame, body, &candidates, app.picker_index);
            draw_footer(frame, footer, &picker_keys(&app.config.keys), None);
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

/// A piece of the right side of the header, with its style.
type Part = (String, Style);

fn parts_width(parts: &[Part]) -> usize {
    parts.iter().map(|(text, _)| text.width()).sum()
}

/// The header. `rights` are the texts for the right side, the preferred one first. The first
/// that fits stays.
fn draw_header(frame: &mut Frame, area: Rect, app: &App, view: &str, rights: &[Vec<Part>]) {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let left = format!("Annorex · {view} · {}", app.state.header_text());
    let width = usize::from(area.width);
    let line = match rights.iter().find(|parts| left.width() + 1 + parts_width(parts) <= width) {
        Some(parts) => {
            let gap = width - left.width() - parts_width(parts);
            let mut spans = vec![Span::styled(left, bold), Span::raw(" ".repeat(gap))];
            spans.extend(parts.iter().map(|(text, style)| Span::styled(text.clone(), *style)));
            Line::from(spans)
        }
        None => Line::styled(fit(&left, area.width), bold),
    };
    frame.render_widget(Paragraph::new(line), area);
}

/// `HH:MM` in the time zone of the app for an RFC 3339 time.
fn clock(at: &str, zone: &jiff::tz::TimeZone) -> String {
    at.parse::<jiff::Timestamp>()
        .map_or_else(|_| at.to_owned(), |time| time.to_zoned(zone.clone()).strftime("%H:%M").to_string())
}

/// The send button: `[Send N ▸]`, with the effect of a message while the agent works, or
/// `[Sent HH:MM]` after a send. Dim when the agent shows a dialog.
fn send_button(app: &App) -> Option<Part> {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    if let Some(at) = app.shown_sent() {
        return Some((format!("[Sent {}]", clock(at, &app.zone)), bold));
    }
    let count = app.annotation_count();
    let note = app.review().is_some_and(|review| !review.note.trim().is_empty());
    if count == 0 && !note {
        return None;
    }
    let effect = match (&app.state.safety, app.state.agent) {
        (Safety::Clear { state: Some(state) }, Some(Agent::ClaudeCode)) if state == "working" => " · queued",
        (Safety::Clear { state: Some(state) }, Some(Agent::Pi | Agent::OpenCode)) if state == "working" => {
            " · steers"
        }
        _ => "",
    };
    let style = if matches!(app.state.safety, Safety::Blocked { .. }) {
        Style::new().add_modifier(Modifier::DIM)
    } else {
        bold
    };
    Some((format!("[Send {count}{effect} ▸]"), style))
}

fn draw_footer(frame: &mut Frame, area: Rect, keys: &str, message: Option<String>) {
    let (text, style) = match message {
        Some(message) => (message, Style::new().add_modifier(Modifier::BOLD)),
        None => (keys.to_owned(), Style::new().add_modifier(Modifier::DIM)),
    };
    frame.render_widget(Paragraph::new(fit(&text, area.width)).style(style), area);
}

/// The title follows the document on the screen.
fn title_of(app: &App) -> &'static str {
    if app.shown_file().is_some() || (app.shown_reply_id().is_none() && app.has_root()) {
        "files"
    } else {
        "last reply"
    }
}

/// The question in the footer for a pending confirmation.
fn prompt_text(app: &App, what: Confirm) -> String {
    match what {
        Confirm::Annotation(id) => {
            let number = app.review().and_then(|review| review.in_order().iter().position(|a| a.id == id));
            format!("Delete annotation {}? y/n", number.map_or(0, |index| index + 1))
        }
        Confirm::Discard if app.review().is_none_or(Review::is_empty) => {
            "Remove the draft file that Annorex cannot read? y/n".to_owned()
        }
        Confirm::Discard if app.shown_file().is_some() => "Discard the notes of this file? y/n".to_owned(),
        Confirm::Discard => "Discard the notes of this reply? y/n".to_owned(),
        Confirm::CloseFile => {
            let path = app.shown_file().map_or_else(String::new, |path| path.display().to_string());
            format!("Close {path}? It has unsent notes. y/n")
        }
        Confirm::Quit if app.unsent_file_count() > 1 => {
            let count = app.unsent_file_count();
            let files = "documents";
            match &app.save_error {
                Some(error) => format!(
                    "Unsent notes in {count} {files}. The drafts cannot be saved: {error}. s send · d discard · esc back"
                ),
                None => format!(
                    "Unsent notes in {count} {files}. s send · k keep the drafts · d discard · esc back"
                ),
            }
        }
        Confirm::Quit => match &app.save_error {
            Some(error) => {
                format!("Unsent notes. The draft cannot be saved: {error}. s send · d discard · esc back")
            }
            None => "Unsent notes. s send · k keep the draft · d discard · esc back".to_owned(),
        },
    }
}

/// Where the document on the screen is: `reply 2/5`, or `file 1/3` among the open files.
fn position_text(app: &App) -> String {
    let (selected, count) = app.reply_position();
    if count == 0 {
        return if app.has_root() { "no file open" } else { "reply 1/0" }.to_owned();
    }
    let open_files = app.files.len();
    if app.shown_file().is_some() {
        let before =
            app.replies.iter().take(selected).filter(|reply| app.files.contains_key(&reply.id)).count();
        format!("file {}/{open_files}", before + 1)
    } else {
        format!("reply {}/{}", selected + 1, count - open_files)
    }
}

fn draw_viewer(frame: &mut Frame, app: &mut App, layout: Layout, header: Rect, footer: Rect) {
    let files = app.has_root();
    let position = position_text(app);
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let unsent = app.unsent_replies();
    let button = send_button(app);
    let with_button = |text: String| -> Vec<Part> {
        let mut parts = vec![(text, bold)];
        if let Some(button) = button.clone() {
            parts.push(("  ".to_owned(), bold));
            parts.push(button);
        }
        parts
    };
    let mut rights = Vec::new();
    if !unsent.is_empty() {
        let numbers: Vec<String> = unsent.iter().map(ToString::to_string).collect();
        rights.push(with_button(format!("{position} · unsent {}", numbers.join(","))));
    }
    rights.push(with_button(position.clone()));
    rights.push(vec![(position, bold)]);
    draw_header(frame, header, app, title_of(app), &rights);
    let (doc_area, panel_area, overlay) = match layout {
        Layout::Single { doc, overlay, .. } => (doc, app.panel_open.then_some(overlay), true),
        Layout::Bottom { doc, panel, .. } | Layout::Side { doc, panel, .. } => {
            (doc, app.panel_open.then_some(panel), false)
        }
        Layout::TooSmall => return,
    };
    app.overlay = overlay;
    let doc_area = match app.banner() {
        Some(text) if doc_area.height > 1 => {
            let row = Rect::new(doc_area.x, doc_area.y, doc_area.width, 1);
            let style = Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD);
            frame.render_widget(Paragraph::new(fit(&text, doc_area.width)).style(style), row);
            Rect::new(doc_area.x, doc_area.y + 1, doc_area.width, doc_area.height - 1)
        }
        _ => doc_area,
    };
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
            PanelView::Tree => draw_tree(frame, app, panel, borders),
            PanelView::Finder => draw_finder(frame, app, panel, borders),
        }
    }
    draw_editor(frame, app, doc_area);
    let prompt = app.confirm.map(|what| prompt_text(app, what));
    let message = prompt
        .or_else(|| app.find_input.as_ref().map(|input| format!("/{input}")))
        .or_else(|| app.status.clone())
        .or_else(|| app.save_error.as_ref().map(|error| format!("draft not saved: {error}")))
        .or_else(|| {
            (app.panel_open && app.panel_view == PanelView::Tree && app.tree.is_truncated())
                .then(|| format!("The tree shows the first {} entries.", thousands(TREE_LIMIT)))
        });
    let keys = if app.editor.is_some() {
        EDITOR_KEYS.to_owned()
    } else if app.focus == Focus::Panel && app.panel_view == PanelView::Annotations {
        annotation_keys(&app.config.keys)
    } else if app.focus == Focus::Panel && app.panel_view == PanelView::Tree {
        tree_keys(&app.config.keys)
    } else if app.focus == Focus::Panel && app.panel_view == PanelView::Finder {
        FINDER_KEYS.to_owned()
    } else if app.focus == Focus::Panel {
        panel_keys(&app.config.keys)
    } else if files {
        files_keys(&app.config.keys)
    } else {
        viewer_keys(&app.config.keys)
    };
    draw_footer(frame, footer, &keys, message);
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
    // A wrap that dropped its space needs one here. A wrap that kept it does not.
    let mut joined = String::new();
    let mut begin = 0;
    let mut mine = (0, 0);
    for (position, text) in texts.iter().enumerate() {
        let length = text.chars().count();
        if start + position == index {
            mine = (begin, begin + length);
        }
        joined.push_str(text);
        begin += length;
        if !text.ends_with(' ') {
            joined.push(' ');
            begin += 1;
        }
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

/// The lines of a list panel: the first line is a header, and the rest scrolls with the cursor.
fn draw_list(frame: &mut Frame, inner: Rect, header: Option<String>, rows: &[String], cursor: usize) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(header) = header {
        lines.push(Line::styled(fit(&header, inner.width), Style::new().add_modifier(Modifier::BOLD)));
    }
    let height = usize::from(inner.height).saturating_sub(lines.len());
    let offset = cursor.saturating_sub(height.saturating_sub(1));
    for (index, text) in rows.iter().enumerate().skip(offset).take(height) {
        let style =
            if index == cursor { Style::new().add_modifier(Modifier::REVERSED) } else { Style::new() };
        lines.push(Line::styled(fit(text, inner.width), style));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_tree(frame: &mut Frame, app: &App, area: Rect, borders: Borders) {
    let block = Block::new().borders(borders).title(" Tree ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if !app.tree.is_loaded() {
        frame.render_widget(Paragraph::new(fit("Scanning…", inner.width)), inner);
        return;
    }
    let filtering = app.tree.filter().is_some_and(|text| !text.is_empty());
    let rows: Vec<String> = app
        .tree
        .visible()
        .iter()
        .map(|row| {
            let path = &row.entry.path;
            let indent = "  ".repeat(row.depth);
            if row.entry.is_dir {
                let arrow = if app.tree.is_expanded(path) { "▾" } else { "▸" };
                let name =
                    path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
                format!("  {indent}{arrow} {name}/")
            } else {
                let dot = if app.is_marked(path) { "●" } else { " " };
                let changed = if app.tree.is_changed(path) { "*" } else { " " };
                let name = if filtering || row.changed_section {
                    path.display().to_string()
                } else {
                    path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
                };
                format!("  {indent}{dot}{changed}{name}")
            }
        })
        .collect();
    let header = app.tree.filter().map(|text| format!("/{text}")).or_else(|| {
        let count = app.tree.changed_count();
        (count > 0).then(|| format!("* changed by the agent: {count}"))
    });
    if rows.is_empty() {
        let note = if header.is_some() { "no match" } else { "no files" };
        let mut lines = Vec::new();
        if let Some(header) = header {
            lines.push(Line::styled(fit(&header, inner.width), Style::new().add_modifier(Modifier::BOLD)));
        }
        lines.push(Line::raw(fit(note, inner.width)));
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }
    draw_list(frame, inner, header, &rows, app.tree.index);
}

fn draw_finder(frame: &mut Frame, app: &App, area: Rect, borders: Borders) {
    let block = Block::new().borders(borders).title(" Find file ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let rows: Vec<String> = app
        .finder
        .results()
        .map(|path| {
            let dot = if app.is_marked(std::path::Path::new(path)) { "●" } else { " " };
            format!("{dot} {path}")
        })
        .collect();
    let header = Some(format!("> {}", app.finder.query));
    if rows.is_empty() {
        let lines = vec![
            Line::styled(
                fit(&format!("> {}", app.finder.query), inner.width),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Line::raw(fit(if app.tree.is_loaded() { "no match" } else { "Scanning…" }, inner.width)),
        ];
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }
    draw_list(frame, inner, header, &rows, app.finder.index);
}

fn draw_reply_list(frame: &mut Frame, app: &App, area: Rect, borders: Borders) {
    let has_replies = app.replies.iter().any(|reply| !app.files.contains_key(&reply.id));
    let title = match (app.has_root(), has_replies) {
        (false, _) => " Replies ",
        (true, false) => " Files ",
        (true, true) => " Documents ",
    };
    let block = Block::new().borders(borders).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if let ReplySource::Pick(candidates) = &app.reply_source {
        draw_picker(frame, inner, candidates, app.picker_index);
        return;
    }
    let reason: Vec<Line<'static>> = match &app.reply_source {
        ReplySource::Missing(message) => {
            vec![Line::raw(fit(message, inner.width)), Line::raw(fit("r looks again", inner.width))]
        }
        _ => Vec::new(),
    };
    let height = usize::from(inner.height).saturating_sub(reason.len()).max(1);
    let offset = app.panel_index.saturating_sub(height.saturating_sub(1));
    let lines: Vec<Line<'static>> = app
        .replies
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, reply)| {
            let files = app.files.contains_key(&reply.id);
            let marker = if index == app.selected { "▸" } else { " " };
            let time = short_time(reply.at.as_deref(), &app.zone);
            let unsent = app.reviews.get(&reply.id).is_some_and(Review::has_unsent);
            let dot = if files && app.changes.contains_key(&reply.id) {
                "≠"
            } else if unsent {
                "●"
            } else {
                " "
            };
            let text = if files {
                let path = app.files.get(&reply.id).map_or_else(
                    || reply.first_line().to_owned(),
                    |file| file.relative.display().to_string(),
                );
                let count = app.reviews.get(&reply.id).map_or(0, |review| review.annotations.len());
                format!("{marker} {:>2}{dot} {path} ({count})", index + 1)
            } else {
                format!("{marker} {:>2}{dot}{time} {}", index + 1, reply.first_line())
            };
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
    let lines: Vec<Line<'static>> = reason.into_iter().chain(lines).collect();
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
    let items = app.review().map(Review::in_order).unwrap_or_default();
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
    let fixed = 2 + lines.len() - typed.split('\n').count();
    let height = u16::try_from(text_rows.min(6) + fixed).unwrap_or(u16::MAX).min(area.height);
    let rect = Rect::new(area.x, area.bottom() - height, area.width, height);
    let prefix = if app.editing_id.is_some() { "Edit " } else { "" };
    let title = if editor.general {
        " Note ".to_owned()
    } else {
        format!(" {prefix}{} · l.{} ", kind_word(editor.kind), app.doc.line_of(editor.range.start))
    };
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
