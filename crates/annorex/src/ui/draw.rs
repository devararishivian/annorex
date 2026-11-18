//! Drawing of the viewer with ratatui.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, BoxView, Confirm, Docked, Focus, ReplySource, Screen, StartView, match_ranges};
use super::bar::{draw_action_bar, shown_rows};
use super::cards::{Side, draw_pick, draw_side, kind_color, range_style, rank, split_markers};
use super::editor::MAX_ROWS;
use super::focus::{Mode, View};
use super::hits::Hit;
use crate::agent::Agent;
use crate::doc::{Doc, DocKind, DocSpan, SpanStyle};
use crate::files::TREE_LIMIT;
use crate::keys::Action;
use crate::layout::{Layout, RAIL_WIDTH, compute};
use crate::replies::Candidate;
use crate::review::{Kind, Review};
use crate::safety::Safety;
use crate::wrap::{VisualLine, fit, gutter_width};

/// One hint of the footer: the action that a click does, when there is one, and the text.
pub(super) type Hint = (Option<Action>, String);

/// The text between two hints of the footer.
const HINT_GAP: &str = " · ";

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

/// The hints of the footer for the current mode, most important first. They show the keys that are
/// bound now. `? help` is last wherever the help key works.
#[allow(clippy::too_many_lines, reason = "one list of hints for each mode")]
pub(super) fn footer_hints(app: &App) -> Vec<Hint> {
    let k = |action| app.config.keys.label(action);
    let hint = |action: Action, text: String| (Some(action), text);
    let plain = |text: &str| (None, text.to_owned());
    let help = hint(Action::Help, format!("{} help", k(Action::Help)));
    let up_down = format!("{}/{}", k(Action::Down), k(Action::Up));
    let notes = format!(
        "{} {} {} {} note",
        k(Action::LooksGood),
        k(Action::Comment),
        k(Action::Delete),
        k(Action::Replace)
    );
    match &app.screen {
        Screen::Error(_) => {
            return vec![
                hint(Action::Reload, format!("{} try again", k(Action::Reload))),
                hint(Action::Quit, format!("{} quit", k(Action::Quit))),
            ];
        }
        Screen::Picker(_) => {
            return vec![
                hint(Action::Down, format!("{up_down} move")),
                plain("enter open"),
                hint(Action::Quit, format!("{} quit", k(Action::Quit))),
            ];
        }
        Screen::Viewer => {}
    }
    if app.editor.is_some() {
        let newline = if app.shift_enter { "shift+enter new line" } else { "ctrl+j new line" };
        return vec![plain("enter save"), plain(newline), plain("esc cancel")];
    }
    if app.pick.is_some() {
        return vec![plain("1-9 choose"), plain("esc close")];
    }
    // The keys of the document on the screen: a file has the file keys, a reply has the reply keys.
    // With no document on the screen, the view that the viewer started as decides.
    let file_keys = app.has_root()
        && match app.shown_file() {
            Some(_) => true,
            None => app.shown_id().is_none() && app.start_view() == StartView::Files,
        };
    // `S send`, `P paste`, and `Y copy` stay together in each footer that has them.
    let send_keys = || {
        [
            hint(Action::Send, format!("{} send", k(Action::Send))),
            hint(Action::Paste, format!("{} paste", k(Action::Paste))),
            hint(Action::Copy, format!("{} copy", k(Action::Copy))),
        ]
    };
    let lists = |out: &mut Vec<Hint>| {
        out.extend(send_keys());
        out.push(hint(Action::Annotations, format!("{} notes", k(Action::Annotations))));
        let list = hint(Action::Replies, format!("{} list", k(Action::Replies)));
        let tree = hint(Action::Tree, format!("{} tree", k(Action::Tree)));
        if file_keys {
            out.extend([tree, hint(Action::FindFile, format!("{} find", k(Action::FindFile))), list]);
        } else if app.has_root() {
            out.extend([list, tree]);
        } else {
            out.push(list);
        }
    };
    let mut out = match app.focus {
        Focus::Box if app.boxed == Some(BoxView::Finder) => {
            return vec![
                plain("type to filter"),
                plain("enter open"),
                plain("ctrl+n/ctrl+p move"),
                plain("esc close"),
            ];
        }
        Focus::Box if app.start_box => {
            vec![hint(Action::Down, format!("{up_down} move")), plain("enter open"), plain("esc quit")]
        }
        Focus::Box => {
            vec![hint(Action::Down, format!("{up_down} move")), plain("enter open"), plain("esc close")]
        }
        Focus::Tree => {
            let mut out = vec![
                hint(Action::Down, format!("{up_down} move")),
                plain("enter open"),
                hint(Action::Left, format!("{}/{} close/open", k(Action::Left), k(Action::Right))),
                hint(Action::Find, format!("{} filter", k(Action::Find))),
                plain("esc back"),
                hint(Action::Hidden, format!("{} hidden", k(Action::Hidden))),
            ];
            // The send keys come after `esc back` and `. hidden`, so a narrow footer keeps the way back first.
            out.extend(send_keys());
            out.push(hint(Action::Tree, format!("{} hide", k(Action::Tree))));
            out
        }
        Focus::Rail => {
            let mut out = vec![
                hint(Action::Down, format!("{up_down} note")),
                plain("enter go"),
                hint(Action::Edit, format!("{} edit", k(Action::Edit))),
                hint(Action::Remove, format!("{} remove", k(Action::Remove))),
                plain("esc back"),
            ];
            out.extend(send_keys());
            out
        }
        Focus::Doc => match app.mode {
            Mode::Browse => {
                let mut out = vec![hint(Action::Down, format!("{up_down} choose a block"))];
                lists(&mut out);
                out
            }
            Mode::Block(_) => {
                let mut out = vec![hint(Action::Comment, notes)];
                lists(&mut out);
                // A narrow footer keeps the essential hints first: the note keys and the send keys. The
                // moves and the other hints follow, in order of use.
                out.insert(4.min(out.len()), hint(Action::Down, format!("{up_down} block")));
                out.insert(5.min(out.len()), plain("esc unselect"));
                out.insert(6.min(out.len()), hint(Action::Select, format!("{} text", k(Action::Select))));
                out.insert(
                    7.min(out.len()),
                    hint(Action::Edit, format!("{}/{} edit", k(Action::Edit), k(Action::Remove))),
                );
                // The least important hint: a narrow footer leaves it out first.
                out.push(plain("enter actions"));
                out
            }
            Mode::Text if app.selection.is_some() => {
                let mut out = vec![
                    hint(Action::Comment, notes),
                    hint(Action::Select, format!("{} end", k(Action::Select))),
                    plain("esc unselect"),
                ];
                out.extend(send_keys());
                out
            }
            Mode::Text => {
                let moves = [Action::Left, Action::Down, Action::Up, Action::Right].map(k).join("/");
                let mut out = vec![hint(Action::Comment, notes)];
                lists(&mut out);
                // The essential hints first, as in block mode.
                out.insert(4.min(out.len()), hint(Action::Down, format!("{moves} move")));
                out.insert(5.min(out.len()), hint(Action::Select, format!("{} select", k(Action::Select))));
                out.insert(6.min(out.len()), plain("esc unselect"));
                out
            }
        },
    };
    out.push(help);
    // `q quit` ends the footer of the document views. The footer keeps it before all other hints.
    if matches!(app.focus, Focus::Doc | Focus::Tree | Focus::Rail) {
        out.push(hint(Action::Quit, format!("{} quit", k(Action::Quit))));
    }
    out
}

/// Draw the whole viewer.
pub fn draw(frame: &mut Frame, app: &mut App) {
    app.hits.clear();
    let area = frame.area();
    let layout = compute(area, app.has_root(), app.tree_open);
    let Some((header, footer)) = layout.header_footer() else {
        app.docked = Docked::default();
        draw_too_small(frame, area);
        return;
    };
    let body = Rect::new(area.x, header.bottom(), area.width, footer.y - header.bottom());
    match app.screen.clone() {
        Screen::Error(message) => {
            draw_header(
                frame,
                app,
                header,
                &[format!("Annorex · {} · {}", title_of(app), app.state.header_text())],
                None,
            );
            let paragraph = Paragraph::new(format!("Annorex: {message}")).wrap(Wrap { trim: true });
            frame.render_widget(paragraph, body);
            draw_footer(frame, app, footer, None, footer_hints(app));
        }
        Screen::Picker(candidates) => {
            draw_header(
                frame,
                app,
                header,
                &[format!("Annorex · choose the session · {}", app.state.header_text())],
                None,
            );
            let rect = open_box(frame, app, area, body, candidates.len() + 2);
            let block = Block::new().borders(Borders::ALL).title(" Sessions ");
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            draw_picker(frame, app, inner, &candidates, app.picker_index);
            draw_footer(frame, app, footer, None, footer_hints(app));
        }
        Screen::Viewer => {
            draw_viewer(frame, app, layout, header, footer);
            if app.help_open {
                draw_help(frame, area, app);
            }
        }
    }
}

/// The rectangle of `width` × `height` in the middle of `area`. A box larger than `area` gets the size
/// of `area`.
pub(super) fn centered_box(area: Rect, width: u16, height: u16) -> Rect {
    let (width, height) = (width.min(area.width), height.min(area.height));
    Rect::new(area.x + (area.width - width) / 2, area.y + (area.height - height) / 2, width, height)
}

/// The most rows of a centered box, with its borders.
const BOX_ROWS: u16 = 20;

/// Clear the centered box over `body` and put it in the hit map: a click outside it reaches nothing
/// below. `screen` is the whole frame. The box is high enough for `rows` rows of content, up to
/// `BOX_ROWS`.
fn open_box(frame: &mut Frame, app: &mut App, screen: Rect, body: Rect, rows: usize) -> Rect {
    let width = body.width.saturating_sub(4).min(76);
    let wanted = u16::try_from(rows).unwrap_or(u16::MAX).saturating_add(2).clamp(6, BOX_ROWS);
    let height = body.height.saturating_sub(2).min(wanted);
    let rect = centered_box(body, width, height);
    frame.render_widget(Clear, rect);
    app.hits.push(screen, Hit::BoxOutside);
    app.hits.push(rect, Hit::BoxInside);
    rect
}

/// The groups of the help screen. The file group shows only when the viewer has a work directory.
const HELP_MOVE: &[Action] = &[
    Action::Down,
    Action::Up,
    Action::Left,
    Action::Right,
    Action::WordNext,
    Action::WordPrev,
    Action::LineStart,
    Action::LineEnd,
    Action::HalfPageDown,
    Action::HalfPageUp,
    Action::Top,
    Action::Bottom,
];
const HELP_NOTE: &[Action] = &[
    Action::Select,
    Action::SelectLines,
    Action::Comment,
    Action::LooksGood,
    Action::Delete,
    Action::Replace,
    Action::Label,
];
const HELP_SEND: &[Action] =
    &[Action::Send, Action::SendWithNote, Action::Paste, Action::Copy, Action::Discard];
const HELP_LISTS: &[Action] =
    &[Action::Replies, Action::Annotations, Action::Edit, Action::Remove, Action::Reload];
const HELP_FILES: &[Action] =
    &[Action::Tree, Action::Hidden, Action::FindFile, Action::PrevFile, Action::NextFile, Action::CloseFile];
const HELP_OTHER: &[Action] = &[Action::Find, Action::FindNext, Action::FindPrev, Action::Help, Action::Quit];

/// The lines of one help column: a bold title, then one line for each key.
fn help_column(app: &App, groups: &[(&str, &[Action])]) -> Vec<Line<'static>> {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let mut lines = Vec::new();
    for (index, (title, actions)) in groups.iter().enumerate() {
        if index > 0 {
            lines.push(Line::default());
        }
        lines.push(Line::styled((*title).to_owned(), bold));
        for action in *actions {
            let key = app.config.keys.label(*action);
            lines.push(Line::from(vec![Span::styled(format!("{key:>7}  "), bold), Span::raw(action.help())]));
        }
    }
    lines
}

/// The help screen: every key from the live key map, over the viewer. Any key closes it.
fn draw_help(frame: &mut Frame, area: Rect, app: &mut App) {
    let mut right: Vec<(&str, &[Action])> = vec![("Send", HELP_SEND), ("Lists", HELP_LISTS)];
    if app.has_root() {
        right.push(("Files", HELP_FILES));
    }
    right.push(("Other", HELP_OTHER));
    let mut left = help_column(app, &[("Move", HELP_MOVE), ("Select and annotate", HELP_NOTE)]);
    // `enter` is not a key of the config, so it has a fixed line: the first one of its group, after the
    // title and the blank line that end the group before it.
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let enter =
        Line::from(vec![Span::styled(format!("{:>7}  ", "enter"), bold), Span::raw("show the action bar")]);
    left.insert((HELP_MOVE.len() + 3).min(left.len()), enter);
    let mut right = help_column(app, &right);
    // `esc` is not a key of the config either. It steps back, and it quits with nothing selected.
    for (key, text) in [("esc", "step back; with nothing"), ("", "selected, it quits")] {
        right.push(Line::from(vec![Span::styled(format!("{key:>7}  "), bold), Span::raw(text)]));
    }
    let two = area.width >= 78;
    let rows = if two { left.len().max(right.len()) } else { left.len() + 1 + right.len() };
    let width = if two { 80 } else { 44 };
    let height = u16::try_from(rows + 3).unwrap_or(u16::MAX);
    let rect = centered_box(area, width, height);
    frame.render_widget(Clear, rect);
    app.hits.push(area, Hit::BoxOutside);
    app.hits.push(rect, Hit::BoxInside);
    let block = Block::default().borders(Borders::ALL).title(" Keys ").title_bottom(" any key closes ");
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if two {
        let half = inner.width / 2;
        frame.render_widget(Paragraph::new(left), Rect::new(inner.x + 1, inner.y, half - 1, inner.height));
        frame.render_widget(
            Paragraph::new(right),
            Rect::new(inner.x + half, inner.y, inner.width - half, inner.height),
        );
    } else {
        let mut lines = left;
        lines.push(Line::default());
        lines.extend(right);
        frame.render_widget(
            Paragraph::new(lines),
            Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(1), inner.height),
        );
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

fn text_width(text: &str) -> u16 {
    u16::try_from(text.width()).unwrap_or(u16::MAX)
}

/// The header: the view, the agent, and the position on the left, the send button on the right.
/// `lefts` are the texts for the left side, the preferred one first. The first that fits beside the
/// button stays. When none fits, the last one is cut. The button stays while it fits.
fn draw_header(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    lefts: &[String],
    button: Option<(String, Style)>,
) {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let shortest = lefts.last().cloned().unwrap_or_default();
    let Some((label, style)) = button.filter(|(label, _)| text_width(label) + 11 <= area.width) else {
        frame.render_widget(Paragraph::new(Line::styled(fit(&shortest, area.width), bold)), area);
        return;
    };
    let room = area.width - text_width(&label) - 1;
    let left =
        lefts.iter().find(|left| text_width(left) <= room).cloned().unwrap_or_else(|| fit(&shortest, room));
    let gap = usize::from(area.width - text_width(&left) - text_width(&label));
    let line = Line::from(vec![
        Span::styled(left, bold),
        Span::raw(" ".repeat(gap)),
        Span::styled(label.clone(), style),
    ]);
    frame.render_widget(Paragraph::new(line), area);
    let width = text_width(&label);
    app.hits.push(Rect::new(area.right() - width, area.y, width, 1), Hit::Send);
}

/// `HH:MM` in the time zone of the app for an RFC 3339 time.
fn clock(at: &str, zone: &jiff::tz::TimeZone) -> String {
    at.parse::<jiff::Timestamp>()
        .map_or_else(|_| at.to_owned(), |time| time.to_zoned(zone.clone()).strftime("%H:%M").to_string())
}

/// The send button: `[ Send N → AGENT ▸ ]`, with the effect of a message while the agent works, or
/// `[ Sent HH:MM ]` after a send. Dim with no notes, and when the agent shows a dialog.
fn send_button(app: &App) -> (String, Style) {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let dim = Style::new().add_modifier(Modifier::DIM);
    if let Some(at) = app.shown_sent() {
        return (format!("[ Sent {} ]", clock(at, &app.zone)), bold);
    }
    let count = app.unsent_count();
    if count == 0 && !app.has_unsent_note() {
        return ("[ Send ▸ ]".to_owned(), dim);
    }
    let effect = match (&app.state.safety, app.state.agent) {
        (Safety::Clear { state: Some(state) }, Some(Agent::ClaudeCode)) if state == "working" => " · queued",
        (Safety::Clear { state: Some(state) }, Some(Agent::Pi | Agent::OpenCode)) if state == "working" => {
            " · steers"
        }
        _ => "",
    };
    let agent =
        app.state.agent.map_or_else(String::new, |agent| format!(" → {}", agent.host().display_name()));
    let style = if matches!(app.state.safety, Safety::Blocked { .. }) { dim } else { bold };
    (format!("[ Send {count}{effect}{agent} ▸ ]"), style)
}

/// The footer: the status or a message on the left, the hints on the right. The hints that do not fit
/// are left out from the end, but the help hint stays while it fits. Each hint with an action is in the
/// hit map.
fn draw_footer(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    left: Option<(String, Style)>,
    hints: Vec<Hint>,
) {
    let (left, left_style) = left.unwrap_or_default();
    let used = if left.is_empty() { 0 } else { text_width(&left) + 2 };
    let gap = text_width(HINT_GAP);
    let (keep, rest): (Vec<Hint>, Vec<Hint>) =
        hints.into_iter().partition(|(action, _)| matches!(action, Some(Action::Help | Action::Quit)));
    let mut room = area.width.saturating_sub(used);
    let mut chosen: Vec<Hint> = Vec::new();
    // `q quit` first, then `? help`: both stay while they fit. They show last, with `q quit` at the end.
    let mut kept: Vec<Hint> = Vec::new();
    let mut keep_hints = keep;
    keep_hints.sort_by_key(|(action, _)| u8::from(*action == Some(Action::Help)));
    for hint in keep_hints {
        let width = text_width(&hint.1) + if kept.is_empty() { 0 } else { gap };
        if width <= room {
            room -= width;
            kept.push(hint);
        }
    }
    kept.sort_by_key(|(action, _)| u8::from(*action == Some(Action::Quit)));
    for hint in rest {
        let width = text_width(&hint.1) + gap;
        if width > room {
            break;
        }
        room -= width;
        chosen.push(hint);
    }
    chosen.extend(kept);
    let hints_width: u16 = chosen.iter().map(|(_, text)| text_width(text)).sum::<u16>()
        + gap * u16::try_from(chosen.len().saturating_sub(1)).unwrap_or(0);
    let left = fit(&left, area.width.saturating_sub(hints_width + u16::from(hints_width > 0) * 2));
    let dim = Style::new().add_modifier(Modifier::DIM);
    let filler = usize::from(area.width.saturating_sub(text_width(&left) + hints_width));
    let mut spans = vec![Span::styled(left, left_style), Span::raw(" ".repeat(filler))];
    let mut x = area.right() - hints_width;
    for (index, (action, text)) in chosen.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(HINT_GAP, dim));
            x += gap;
        }
        let width = text_width(&text);
        if let Some(action) = action {
            app.hits.push(Rect::new(x, area.y, width, 1), Hit::Hint(action));
        }
        x += width;
        spans.push(Span::styled(text, dim));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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
        Confirm::Paste => {
            "Paste the notes into the agent prompt, without Enter? Annorex closes after. y/n".to_owned()
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
    if app.shown_id().is_none() {
        return "no file open".to_owned();
    }
    if app.shown_file().is_some() {
        let before =
            app.replies.iter().take(selected).filter(|reply| app.files.contains_key(&reply.id)).count();
        format!("file {}/{open_files}", before + 1)
    } else {
        format!("reply {}/{}", selected + 1, count - open_files)
    }
}

/// The status on the left of the footer: the focused block or the line of the cursor, and the
/// number of notes of the document.
fn doc_status(app: &App) -> String {
    // The block and the line belong to the document: the tree and the rail show only the notes.
    let place = match app.mode {
        _ if app.focus != Focus::Doc => String::new(),
        Mode::Block(index) => match app.doc.blocks.len() {
            0 => "block 0/0".to_owned(),
            count => format!("block {}/{count}", index.min(count - 1) + 1),
        },
        Mode::Browse => match app.doc.blocks.len() {
            0 => "block 0/0".to_owned(),
            count => format!("block -/{count}"),
        },
        Mode::Text => {
            let lines = app.doc.source.lines().count().max(1);
            format!("line {}/{lines}", app.doc.line_of(app.cursor))
        }
    };
    let notes = match app.review().map_or(0, |review| review.annotations.len()) {
        0 => return place,
        1 => "1 note".to_owned(),
        count => format!("{count} notes"),
    };
    if place.is_empty() { notes } else { format!("{place} · {notes}") }
}

#[allow(clippy::too_many_lines, reason = "one place that puts the parts of the viewer on the screen")]
fn draw_viewer(frame: &mut Frame, app: &mut App, layout: Layout, header: Rect, footer: Rect) {
    let position = position_text(app);
    let state = app.state.header_text();
    let base = format!("Annorex · {} · {state} · {position}", title_of(app));
    let mut unsent = Vec::new();
    let replies = app.unsent_replies();
    if !replies.is_empty() {
        let numbers: Vec<String> = replies.iter().map(ToString::to_string).collect();
        unsent.push(format!("unsent in reply {}", numbers.join(",")));
    }
    match app.unsent_open_files() {
        0 => {}
        1 => unsent.push("unsent in 1 file".to_owned()),
        count => unsent.push(format!("unsent in {count} files")),
    }
    let mut lefts = Vec::new();
    if !unsent.is_empty() {
        lefts.push(format!("{base} · {}", unsent.join(" · ")));
        // The compact form: `unsent 1,3`, `unsent 2 files`, or both.
        let compact: Vec<String> = unsent
            .iter()
            .map(|part| part.replace("unsent in reply ", "").replace("unsent in ", ""))
            .collect();
        let compact = compact.join(" · ");
        lefts.push(format!("{base} · unsent {compact}"));
        // Then without the name of the view, so the unsent notice outlasts the longer agent name.
        lefts.push(format!("{state} · {position} · unsent {compact}"));
    }
    lefts.push(base);
    // A narrow header keeps the position and the agent before the name of the view.
    lefts.push(format!("{state} · {position}"));
    // The short agent name, so a very narrow header still shows the state.
    lefts.push(format!("{} · {position}", app.state.header_text_short()));
    lefts.push(position);
    draw_header(frame, app, header, &lefts, Some(send_button(app)));
    let (body, tree, rail) = match layout {
        Layout::Single { doc, .. } => (doc, None, None),
        Layout::Wide { doc, tree, rail, .. } => (doc, tree, rail),
        Layout::TooSmall => return,
    };
    let narrow = matches!(layout, Layout::Single { .. });
    if tree.is_some() {
        app.tree_seen();
    }
    let doc_area = match app.banner() {
        Some(text) if body.height > 1 => {
            let row = Rect::new(body.x, body.y, body.width, 1);
            let style = Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD);
            frame.render_widget(Paragraph::new(fit(&text, body.width)).style(style), row);
            Rect::new(body.x, body.y + 1, body.width, body.height - 1)
        }
        _ => body,
    };
    // Where the window has no card rail, the annotation list opens at the right of the document.
    let list = (!narrow && rail.is_none() && app.list_open && doc_area.width > RAIL_WIDTH + 20)
        .then(|| Rect::new(doc_area.right() - RAIL_WIDTH, doc_area.y, RAIL_WIDTH, doc_area.height));
    let doc_area = match list {
        Some(list) => Rect { width: doc_area.width - list.width, ..doc_area },
        None => doc_area,
    };
    let (doc_area, side) = match rail {
        // The rail starts at the first document row, so a card lines up with its note.
        Some(rail) => (doc_area, Side::Rail(Rect { y: doc_area.y, height: doc_area.height, ..rail })),
        None => split_markers(doc_area),
    };
    app.docked =
        Docked { tree: tree.is_some(), rail: rail.is_some() || list.is_some(), cards: rail.is_some() };
    // The focus background also fills the marker margin, so it reaches the end of the row.
    let fill_width = match side {
        Side::Markers(column) => doc_area.width + column.width,
        Side::Rail(_) | Side::Nothing => doc_area.width,
    };
    draw_doc(frame, app, doc_area, fill_width);
    if app.shown_id().is_none() && app.has_root() {
        draw_no_document(frame, app, doc_area);
    }
    let (margin, gutter) = margin_and_gutter(&app.doc, doc_area);
    let text_area = Rect::new(
        doc_area.x + margin + gutter,
        doc_area.y,
        doc_area.width.saturating_sub(margin + gutter),
        doc_area.height,
    );
    if let Some(strip) = draw_action_bar(frame, text_area, app) {
        // The strip lies over the text of its row, so the line number of that row goes too.
        frame.render_widget(Clear, Rect::new(text_area.x - gutter, strip.y, gutter, 1));
    }
    if let Side::Rail(rail) = side {
        // The empty space right of a capped layout belongs to the rail pane: a click there does nothing
        // and the wheel scrolls the document.
        app.hits.push(Rect { width: header.right().saturating_sub(rail.x), ..rail }, Hit::RailPane);
    }
    draw_side(frame, app, side);
    if let Some(tree) = tree {
        app.hits.push(tree, Hit::TreePane);
        draw_tree(frame, app, tree, Borders::RIGHT);
    }
    if let Some(list) = list {
        app.hits.push(list, Hit::NotesPane);
        draw_annotation_list(frame, app, list, Borders::LEFT);
    }
    // In one column, the tree and the annotation list cover the document while they have the focus.
    // The cover goes in the hit map first, so no click reaches the document below; the rows of the
    // overlay go in after it and win.
    if narrow && app.focus == Focus::Tree && app.has_root() {
        frame.render_widget(Clear, body);
        app.hits.push(body, Hit::BoxInside);
        draw_tree(frame, app, body, Borders::ALL);
    }
    if narrow && app.focus == Focus::Rail {
        frame.render_widget(Clear, body);
        app.hits.push(body, Hit::BoxInside);
        draw_annotation_list(frame, app, body, Borders::ALL);
    }
    draw_pick(frame, app, text_area);
    draw_editor(frame, app, text_area);
    if let Some(view) = app.boxed {
        let screen = Rect::new(header.x, header.y, header.width, footer.bottom() - header.y);
        let middle = Rect::new(header.x, header.bottom(), header.width, footer.y - header.bottom());
        let rows = match (view, &app.reply_source) {
            (BoxView::Finder, _) => usize::from(BOX_ROWS),
            (BoxView::Replies, ReplySource::Pick(candidates)) => candidates.len() + 2,
            (BoxView::Replies, ReplySource::Missing(_)) => app.replies.len() + 2,
            (BoxView::Replies, ReplySource::Ready) => app.replies.len(),
        };
        let rect = open_box(frame, app, screen, middle, rows);
        match view {
            BoxView::Replies => draw_reply_list(frame, app, rect),
            BoxView::Finder => draw_finder(frame, app, rect),
        }
    }
    let tree_shown = tree.is_some() || (narrow && app.focus == Focus::Tree);
    let bold = Style::new().add_modifier(Modifier::BOLD);
    // A question and the find input take the keys, so the footer shows no hints with them.
    let question = app
        .confirm
        .map(|what| prompt_text(app, what))
        .or_else(|| app.find_input.as_ref().map(|input| format!("/{input}")));
    if let Some(question) = question {
        draw_footer(frame, app, footer, Some((question, bold)), Vec::new());
        return;
    }
    let message = app
        .status
        .clone()
        .or_else(|| app.save_error.as_ref().map(|error| format!("draft not saved: {error}")))
        .or_else(|| {
            (tree_shown && app.tree.is_truncated())
                .then(|| format!("The tree shows the first {} entries.", thousands(TREE_LIMIT)))
        });
    let left = match message {
        Some(message) => (message, bold),
        None => (doc_status(app), Style::new().add_modifier(Modifier::DIM)),
    };
    let hints = footer_hints(app);
    draw_footer(frame, app, footer, Some(left), hints);
}

/// The empty state of the files view while no document is on the screen.
fn draw_no_document(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let keys = &app.config.keys;
    let text =
        format!("Open a file from the tree ({}), or find one ({}).", "enter", keys.label(Action::FindFile));
    let row = Rect::new(area.x, area.y + area.height / 2, area.width, 1);
    let style = Style::new().add_modifier(Modifier::DIM);
    frame
        .render_widget(Paragraph::new(fit(&text, area.width)).style(style).alignment(Alignment::Center), row);
}

/// Draw the document in `area`. The focus background fills `fill_width` columns from the left of `area`.
fn draw_doc(frame: &mut Frame, app: &mut App, area: Rect, fill_width: u16) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let height = usize::from(area.height);
    app.page = height;
    let (margin, gutter) = margin_and_gutter(&app.doc, area);
    let text_width = area.width - margin - gutter;
    let cursor_line = app.cursor_line();
    let annotated: Vec<(std::ops::Range<usize>, Kind)> = app
        .review()
        .map(|review| {
            review.annotations.iter().filter(|a| !a.detached).map(|a| (a.range.clone(), a.kind)).collect()
        })
        .unwrap_or_default();
    let selection = app.selection_range();
    app.width = text_width;
    app.text_area = Rect::new(area.x + margin + gutter, area.y, text_width, area.height);
    let focused = match app.mode {
        Mode::Block(index) => app.doc.blocks.get(index).cloned(),
        Mode::Browse | Mode::Text => None,
    };
    let block_mode = matches!(app.mode, Mode::Block(_) | Mode::Browse);
    let rows = app.cache.get(&app.doc, text_width);
    let first_row = |doc_line: usize| rows.iter().position(|row| row.doc_line >= doc_line);
    let mut top_row = first_row(app.top).unwrap_or(0);
    if app.scroll.doc || app.mode == Mode::Browse {
        // The wheel moved the view, or no block has the focus: the view follows nothing.
    } else if let Some(block) = &focused {
        // Block mode: the focused block stays in view. A tall block can scroll inside.
        top_row = View { rows, top: top_row, height }.top_for(top_row, block);
    } else if let Some(cursor_row) = first_row(cursor_line) {
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
    app.top_row = top_row;
    let query = app.query.clone();
    // The cursor cell: the first row of the cursor line that shows the cursor stop.
    // Block mode has no cursor cell: the focus bar is the only focus mark.
    let cursor_cell = (app.focus == Focus::Doc && !block_mode)
        .then(|| {
            let start = first_row(cursor_line)?;
            rows.iter()
                .enumerate()
                .skip(start)
                .take_while(|(_, row)| row.doc_line == cursor_line)
                .find_map(|(index, row)| row.cell_of(app.cursor).map(|cell| (index, cell)))
        })
        .flatten();
    let mut group: RowMatches = None;
    let lines: Vec<Line<'static>> = rows
        .iter()
        .skip(top_row)
        .take(height)
        .enumerate()
        .map(|(offset, row)| {
            let matches = row_matches(&mut group, rows, top_row + offset, query.as_deref());
            let number_style = if !block_mode && row.doc_line == cursor_line {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::new().add_modifier(Modifier::DIM)
            };
            let number = match app.doc.lines.get(row.doc_line) {
                Some(line) if row.first && !line.is_blank() => line.line.to_string(),
                _ => String::new(),
            };
            let number_width = usize::from(gutter.saturating_sub(1));
            let in_focus = focused.as_ref().is_some_and(|block| block.lines.contains(&row.doc_line));
            let offsets = row.offsets();
            let kinds = note_kinds(&offsets, &annotated);
            let mut spans = vec![margin_span(in_focus, top_kind(&kinds), margin)];
            if gutter > 0 {
                spans.push(Span::styled(format!("{number:>number_width$}"), number_style));
                spans.push(Span::raw(" "));
            }
            let cursor = cursor_cell.filter(|&(index, _)| index == top_row + offset).map(|(_, cell)| cell);
            let flags = |ranges: &[std::ops::Range<usize>]| -> Vec<bool> {
                offsets
                    .iter()
                    .map(|at| at.is_some_and(|at| ranges.iter().any(|range| range.contains(&at))))
                    .collect()
            };
            let marks = Marks { kinds, selected: flags(selection.as_slice()) };
            spans.extend(row_spans(row, &matches, cursor, &marks));
            Line::from(spans)
        })
        .collect();
    if let Some(block) = &focused {
        fill_focus(
            frame,
            Rect { width: fill_width, ..area },
            rows.iter().skip(top_row).take(height).map(|row| block.lines.contains(&row.doc_line)),
        );
    }
    push_doc_hits(app, area, top_row, margin + gutter);
    frame.render_widget(Paragraph::new(lines), area);
}

/// The width of the margin and of the line number gutter in a document area. Column 0 is the
/// margin for the focus bar. Plain text and code keep the line number gutter.
fn margin_and_gutter(doc: &Doc, area: Rect) -> (u16, u16) {
    let margin = 1.min(area.width.saturating_sub(1));
    let gutter = if doc.kind == DocKind::Plain {
        gutter_width(doc).min(area.width.saturating_sub(1 + margin))
    } else {
        0
    };
    (margin, gutter)
}

/// The margin cell of a row: the cyan focus bar in a focused block, else a bar in the color of the
/// note under the row, else blanks.
fn margin_span(in_focus: bool, note: Option<Kind>, margin: u16) -> Span<'static> {
    if margin > 0 && (in_focus || note.is_some()) {
        let color = if in_focus { Color::Cyan } else { note.map_or(Color::Cyan, kind_color) };
        Span::styled("\u{258d}", Style::new().fg(color))
    } else {
        Span::raw(" ".repeat(usize::from(margin)))
    }
}

/// Give the rows of the focused block the focus background over the whole width, also the blank
/// rows inside the block. `inside` tells for each row on the screen if it is in the block.
fn fill_focus(frame: &mut Frame, area: Rect, inside: impl Iterator<Item = bool>) {
    let fill = Style::new().bg(Color::DarkGray);
    for (offset, inside) in inside.enumerate() {
        if inside {
            let y = area.y + u16::try_from(offset).unwrap_or(0);
            frame.buffer_mut().set_style(Rect::new(area.x, y, area.width, 1), fill);
        }
    }
}

/// Push the hit areas of the document rows on the screen. A block reaches the whole row, and the
/// text area lies over it, so a click in the margin or the gutter gives the block and a click in the
/// text gives the cell. `DocCell` holds the index of the visual row and the column where the text starts.
fn push_doc_hits(app: &mut App, area: Rect, top_row: usize, left: u16) {
    let height = usize::from(area.height);
    let text_x = area.x + left;
    let text_width = area.width.saturating_sub(left);
    let rows = app.cache.get(&app.doc, app.width);
    let shown = rows.len().saturating_sub(top_row).min(height);
    let mut block_rects: Vec<(usize, u16, u16)> = Vec::new();
    for offset in 0..shown {
        let Some(row) = rows.get(top_row + offset) else { break };
        let y = area.y + u16::try_from(offset).unwrap_or(0);
        let Some(index) =
            app.doc.blocks.partition_point(|block| block.lines.start <= row.doc_line).checked_sub(1).filter(
                |&index| app.doc.blocks.get(index).is_some_and(|block| block.lines.end > row.doc_line),
            )
        else {
            continue;
        };
        match block_rects.last_mut() {
            Some((last, from, count)) if *last == index && *from + *count == y => *count += 1,
            _ => block_rects.push((index, y, 1)),
        }
    }
    for (index, y, count) in block_rects {
        app.hits.push(Rect::new(area.x, y, area.width, count), Hit::Block(index));
    }
    if text_width == 0 {
        return;
    }
    for offset in 0..shown {
        let row = top_row + offset;
        let y = area.y + u16::try_from(offset).unwrap_or(0);
        app.hits.push(Rect::new(text_x, y, text_width, 1), Hit::DocCell { row, col: text_x });
    }
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

/// The first row of a `DocLine` and the matches of each of its rows.
type RowMatches = Option<(usize, Vec<Vec<std::ops::Range<usize>>>)>;

#[cfg(test)]
thread_local! {
    /// The number of searches over a joined `DocLine` on this thread. A test counts them.
    static MATCH_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static MATCH_RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The matches of `query` in each row of the `DocLine` that has row `index`, as character ranges of
/// each row, with the index of the first row of the group. A match can go over a wrap: the rows of
/// one `DocLine` are joined with the space that the wrap removed. The line is searched once.
fn group_matches(
    rows: &[VisualLine],
    index: usize,
    query: &str,
) -> (usize, Vec<Vec<std::ops::Range<usize>>>) {
    let Some(row) = rows.get(index) else { return (index, Vec::new()) };
    let start = rows.partition_point(|other| other.doc_line < row.doc_line);
    let group: Vec<&VisualLine> =
        rows.iter().skip(start).take_while(|other| other.doc_line == row.doc_line).collect();
    // A wrap that dropped its space needs one here. A wrap that kept it does not.
    let mut joined = String::new();
    let mut spans = Vec::with_capacity(group.len());
    let mut begin = 0;
    for row in &group {
        let text = row.plain();
        let length = text.chars().count();
        spans.push((begin, begin + length));
        joined.push_str(&text);
        begin += length;
        if !text.ends_with(' ') {
            joined.push(' ');
            begin += 1;
        }
    }
    #[cfg(test)]
    MATCH_RUNS.with(|runs| runs.set(runs.get() + 1));
    let matches = match_ranges(&joined, query);
    // The matches are sorted and do not overlap: one index walks them for all rows.
    let mut next = 0;
    let per_row = spans
        .into_iter()
        .map(|(from, to)| {
            while matches.get(next).is_some_and(|range| range.end <= from) {
                next += 1;
            }
            let mut row = Vec::new();
            for range in matches.iter().skip(next).take_while(|range| range.start < to) {
                #[cfg(test)]
                MATCH_VISITS.with(|visits| visits.set(visits.get() + 1));
                row.push(range.start.max(from) - from..range.end.min(to) - from);
            }
            row
        })
        .collect();
    (start, per_row)
}

/// The matches of row `index`. `group` holds the matches of the `DocLine` of the last row: the rows
/// of one `DocLine` follow each other, so each line is searched once for a frame.
fn row_matches(
    group: &mut RowMatches,
    rows: &[VisualLine],
    index: usize,
    query: Option<&str>,
) -> Vec<std::ops::Range<usize>> {
    let Some(query) = query else { return Vec::new() };
    let stale =
        group.as_ref().is_none_or(|(start, per_row)| index < *start || index >= start + per_row.len());
    if stale {
        *group = Some(group_matches(rows, index, query));
    }
    group.as_ref().and_then(|(start, per_row)| per_row.get(index - start)).cloned().unwrap_or_default()
}

/// For each character of a row: the type of the note over it. Where notes of two types overlap,
/// the type with the higher rank shows.
fn note_kinds(offsets: &[Option<usize>], annotated: &[(std::ops::Range<usize>, Kind)]) -> Vec<Option<Kind>> {
    offsets
        .iter()
        .map(|at| {
            let at = (*at)?;
            annotated
                .iter()
                .filter(|(range, _)| range.contains(&at))
                .map(|(_, kind)| *kind)
                .max_by_key(|kind| rank(*kind))
        })
        .collect()
}

/// The type that shows in the margin of a row: the highest rank under any character of the row.
fn top_kind(kinds: &[Option<Kind>]) -> Option<Kind> {
    kinds.iter().flatten().copied().max_by_key(|kind| rank(*kind))
}

/// Which characters of a row are under a note, of which type, and which are selected.
struct Marks {
    kinds: Vec<Option<Kind>>,
    selected: Vec<bool>,
}

/// The spans of a row, with the matches marked, the cursor cell reversed, the text under a note in
/// the style of its type, and the selection reversed.
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
            let noted = match marks.kinds.get(position) {
                Some(Some(kind)) => base.patch(range_style(*kind)),
                _ => base,
            };
            let selected = marks.selected.get(position) == Some(&true);
            if cursor == Some(position) || selected {
                noted.add_modifier(Modifier::REVERSED)
            } else if matches.iter().any(|range| range.contains(&position)) {
                highlight
            } else {
                noted
            }
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

/// The first row on the screen of a list of `count` rows with `height` rows of room: `wanted` when
/// the wheel scrolled the list, else the first row that keeps `cursor` in view.
fn list_top(wanted: Option<usize>, cursor: usize, count: usize, height: usize) -> usize {
    match wanted {
        Some(top) => top.min(count.saturating_sub(height)),
        None => cursor.saturating_sub(height.saturating_sub(1)),
    }
}

/// After a draw, the wheel scroll of a list is the first row that the list shows. A wheel step past the
/// end then builds no hidden rows, so the next step back moves the view at once.
fn settle(wanted: &mut Option<usize>, shown: usize) {
    if wanted.is_some() {
        *wanted = Some(shown);
    }
}

/// The lines of a list: the first line is a header, and the rest scrolls with the cursor, or from
/// `wanted` after the wheel. The result is the index of the first row on the screen and the screen
/// row where it is.
fn draw_list(
    frame: &mut Frame,
    inner: Rect,
    header: Option<String>,
    rows: &[String],
    (cursor, wanted): (usize, Option<usize>),
    marked: bool,
) -> (usize, u16) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(header) = header {
        lines.push(Line::styled(fit(&header, inner.width), Style::new().add_modifier(Modifier::BOLD)));
    }
    let first_y = inner.y + u16::try_from(lines.len()).unwrap_or(0);
    let height = usize::from(inner.height).saturating_sub(lines.len());
    let offset = list_top(wanted, cursor, rows.len(), height);
    for (index, text) in rows.iter().enumerate().skip(offset).take(height) {
        let style = if marked && index == cursor {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        lines.push(Line::styled(fit(text, inner.width), style));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    (offset, first_y)
}

/// Put one hit for each row of a list on the screen, from the row `offset` at the screen row `first_y`.
fn push_rows(
    app: &mut App,
    inner: Rect,
    (offset, first_y): (usize, u16),
    count: usize,
    hit: fn(usize) -> Hit,
) {
    let shown = usize::from(inner.bottom().saturating_sub(first_y));
    for (row, index) in (offset..count).take(shown).enumerate() {
        let y = first_y + u16::try_from(row).unwrap_or(0);
        app.hits.push(Rect::new(inner.x, y, inner.width, 1), hit(index));
    }
}

/// The number of notes in the open files below each directory.
fn notes_by_directory(app: &App) -> HashMap<PathBuf, usize> {
    let mut counts: HashMap<PathBuf, usize> = HashMap::new();
    for (id, file) in &app.files {
        let count = app.reviews.get(id).map_or(0, |review| review.annotations.len());
        if count == 0 {
            continue;
        }
        for parent in file.relative.ancestors().skip(1).filter(|parent| !parent.as_os_str().is_empty()) {
            *counts.entry(parent.to_path_buf()).or_default() += count;
        }
    }
    counts
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

fn draw_tree(frame: &mut Frame, app: &mut App, area: Rect, borders: Borders) {
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
    let counts = notes_by_directory(app);
    let rows: Vec<String> = app
        .tree
        .visible()
        .iter()
        .map(|row| {
            let path = &row.entry.path;
            let indent = "  ".repeat(row.depth);
            if row.entry.is_dir {
                let arrow = if app.tree.is_expanded(path) { "▾" } else { "▸" };
                let count = counts.get(path).map_or_else(String::new, |count| format!(" {count}"));
                format!("  {indent}{arrow} {}/{count}", file_name(path))
            } else {
                let dot = if app.is_marked(path) { "●" } else { " " };
                let changed = if app.tree.is_changed(path) { "*" } else { " " };
                let name = if filtering || row.changed_section {
                    path.display().to_string()
                } else {
                    file_name(path)
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
    // Without the focus, the tree shows no cursor.
    let cursor = (app.tree.index, app.scroll.tree);
    let place = draw_list(frame, inner, header, &rows, cursor, app.focus == Focus::Tree);
    app.tops.tree = place.0;
    settle(&mut app.scroll.tree, place.0);
    push_rows(app, inner, place, rows.len(), Hit::TreeRow);
}

fn draw_finder(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new().borders(Borders::ALL).title(" Find file ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let rows: Vec<String> = app
        .finder
        .results()
        .map(|path| {
            let dot = if app.is_marked(Path::new(path)) { "●" } else { " " };
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
    let place = draw_list(frame, inner, header, &rows, (app.finder.index, app.scroll.boxed), true);
    app.tops.boxed = place.0;
    settle(&mut app.scroll.boxed, place.0);
    push_rows(app, inner, place, rows.len(), Hit::ListRow);
}

fn draw_reply_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let has_replies = app.replies.iter().any(|reply| !app.files.contains_key(&reply.id));
    let title = match (app.has_root(), has_replies) {
        (false, _) => " Replies ",
        (true, false) => " Files ",
        (true, true) => " Documents ",
    };
    let block = Block::new().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if let ReplySource::Pick(candidates) = app.reply_source.clone() {
        draw_picker(frame, app, inner, &candidates, app.picker_index);
        return;
    }
    let reason: Vec<Line<'static>> = match &app.reply_source {
        ReplySource::Missing(message) => {
            vec![Line::raw(fit(message, inner.width)), Line::raw(fit("r looks again", inner.width))]
        }
        _ => Vec::new(),
    };
    let first_y = inner.y + u16::try_from(reason.len()).unwrap_or(0);
    let height = usize::from(inner.height).saturating_sub(reason.len()).max(1);
    let offset = list_top(app.scroll.boxed, app.panel_index, app.replies.len(), height);
    app.tops.boxed = offset;
    settle(&mut app.scroll.boxed, offset);
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
            let style = if index == app.panel_index && app.focus == Focus::Box {
                Style::new().add_modifier(Modifier::REVERSED)
            } else if index == app.selected {
                Style::new().add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            Line::styled(fit(&text, inner.width), style)
        })
        .collect();
    let count = app.replies.len();
    let lines: Vec<Line<'static>> = reason.into_iter().chain(lines).collect();
    frame.render_widget(Paragraph::new(lines), inner);
    push_rows(app, inner, (offset, first_y), count, Hit::ListRow);
}

fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::Comment => "comment",
        Kind::Delete => "delete",
        Kind::LooksGood => "looks good",
        Kind::Replace => "replace",
    }
}

fn draw_annotation_list(frame: &mut Frame, app: &mut App, area: Rect, borders: Borders) {
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
    let offset = list_top(app.scroll.notes, app.annotation_index, items.len(), visible);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut rows: Vec<usize> = Vec::new();
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
        let style = if index == app.annotation_index && app.focus == Focus::Rail {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        lines.push(Line::styled(fit(&first, inner.width), style));
        lines.push(Line::styled(fit(&second, inner.width), style.add_modifier(Modifier::DIM)));
        rows.push(index);
    }
    frame.render_widget(Paragraph::new(lines), inner);
    app.tops.notes = offset;
    settle(&mut app.scroll.notes, offset);
    // Each item has two lines. Both lines reach the item.
    for (position, index) in rows.into_iter().enumerate() {
        let y = inner.y + u16::try_from(position * 2).unwrap_or(u16::MAX);
        let height = 2.min(inner.bottom().saturating_sub(y));
        if height > 0 {
            app.hits.push(Rect::new(inner.x, y, inner.width, height), Hit::ListRow(index));
        }
    }
}

/// The editor box. `area` is the text area of the document. The box has the width of the text
/// and goes directly below the rows of its range. When it does not fit there, it goes directly
/// above them. When it fits in neither place, or the range is not on the screen, it stays in `area`.
/// The text shows in at most `MAX_ROWS` rows. The box follows the end of the text, or shows the rows
/// from the scroll of the editor.
fn draw_editor(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(editor) = app.editor.clone() else { return };
    if area.width < 4 || area.height < 5 {
        return;
    }
    // One column stays free, so a caret at the end of a full row fits.
    let width = usize::from(area.width - 3);
    let rows = super::editor::rows(&editor.text, width);
    let (caret_row, _) = editor.caret(width);
    let top = editor.first_row(rows.len(), caret_row);
    let dim = Style::new().add_modifier(Modifier::DIM);
    let caret = Style::new().add_modifier(Modifier::REVERSED);
    let mut lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(top)
        .take(MAX_ROWS)
        .map(|(index, row)| {
            let text = |range: std::ops::Range<usize>| editor.text.get(range).unwrap_or_default().to_owned();
            if index != caret_row {
                return Line::raw(text(row.clone()));
            }
            // The caret is the reversed cell of the character after it, or a space at the end of the row.
            let at = editor.cursor.clamp(row.start, row.end);
            // A tab or a zero-width character has no cell of its own, so the caret is a space before it.
            let after = editor
                .text
                .get(at..row.end)
                .and_then(|tail| tail.chars().next())
                .filter(|c| !c.is_control() && c.width().unwrap_or(0) > 0);
            let next = after.map_or(at, |c| at + c.len_utf8());
            Line::from(vec![
                Span::raw(text(row.start..at)),
                Span::styled(after.map_or_else(|| " ".to_owned(), String::from), caret),
                Span::raw(text(next..row.end)),
            ])
        })
        .collect();
    if editor.text.is_empty() {
        let hint = match (editor.general, editor.kind) {
            (true, _) => "Write your note",
            (false, Kind::Replace) => "Write the new text",
            (false, _) => "Write your comment",
        };
        lines = vec![Line::from(vec![Span::styled(" ", caret), Span::styled(hint, dim)])];
    }
    let shown = lines.len();
    let height = u16::try_from(shown + 2).unwrap_or(u16::MAX).min(area.height);
    let anchor = if editor.general { None } else { shown_rows(app, &editor.range) };
    let y = editor_top(area, height, anchor);
    let rect = Rect::new(area.x, y, area.width, height);
    if let Some(open) = app.editor.as_mut() {
        open.rows = rows.len();
        open.width = width;
        open.top = top;
        let text_rows = u16::try_from(shown).unwrap_or(u16::MAX).min(height.saturating_sub(2));
        open.area = Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, text_rows);
    }
    let prefix = if app.editing_id.is_some() { "Edit " } else { "" };
    let title = if editor.general {
        " Note ".to_owned()
    } else {
        let word = match editor.kind {
            Kind::Comment => "Comment",
            Kind::Delete => "Remove",
            Kind::LooksGood => "Looks good",
            Kind::Replace => "Replace",
        };
        format!(" {prefix}{word} · line {} ", app.doc.line_of(editor.range.start))
    };
    frame.render_widget(Clear, rect);
    app.hits.push(rect, Hit::Editor);
    let block = Block::new().borders(Borders::ALL).title(title);
    frame.render_widget(Paragraph::new(lines).block(block), rect);
    draw_editor_buttons(frame, app, rect);
}

/// The first row of the editor box of `height` rows. `anchor` is the first and the last row of the
/// range on the screen, as offsets from the top of `area`.
pub(super) fn editor_top(area: Rect, height: u16, anchor: Option<(usize, usize)>) -> u16 {
    let lowest = area.bottom().saturating_sub(height);
    let Some((first, last)) = anchor else { return lowest };
    let offset = |row: usize| area.y.saturating_add(u16::try_from(row).unwrap_or(u16::MAX));
    let below = offset(last + 1);
    let above = offset(first);
    if below.saturating_add(height) <= area.bottom() {
        below
    } else if above >= area.y.saturating_add(height) {
        above - height
    } else {
        below.min(lowest)
    }
}

/// The ` save ` and ` cancel ` buttons on the bottom border of the editor box, at its right end.
fn draw_editor_buttons(frame: &mut Frame, app: &mut App, rect: Rect) {
    const SAVE: &str = " save ";
    const CANCEL: &str = " cancel ";
    let width = |text: &str| u16::try_from(text.width()).unwrap_or(u16::MAX);
    let (save, cancel) = (width(SAVE), width(CANCEL));
    // The corners and one border cell before each button.
    if rect.height < 2 || rect.width < save + cancel + 4 {
        return;
    }
    let y = rect.bottom() - 1;
    let cancel_x = rect.right() - 2 - cancel;
    let save_x = cancel_x - 1 - save;
    let buffer = frame.buffer_mut();
    buffer.set_string(save_x, y, SAVE, Style::new().add_modifier(Modifier::BOLD));
    buffer.set_string(cancel_x, y, CANCEL, Style::new());
    app.hits.push(Rect::new(save_x, y, save, 1), Hit::EditorSave);
    app.hits.push(Rect::new(cancel_x, y, cancel, 1), Hit::EditorCancel);
}

/// `MM-DD HH:MM` in `zone` for an RFC 3339 time. Other text stays as it is.
fn short_time(at: Option<&str>, zone: &jiff::tz::TimeZone) -> String {
    let Some(at) = at else { return String::new() };
    match at.parse::<jiff::Timestamp>() {
        Ok(time) => time.to_zoned(zone.clone()).strftime("%m-%d %H:%M").to_string(),
        Err(_) => at.to_owned(),
    }
}

fn draw_picker(frame: &mut Frame, app: &mut App, area: Rect, candidates: &[Candidate], index: usize) {
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
    let offset = list_top(app.scroll.boxed, index, candidates.len(), rows);
    app.tops.boxed = offset;
    settle(&mut app.scroll.boxed, offset);
    for (position, candidate) in candidates.iter().enumerate().skip(offset).take(rows) {
        let marker = if position == index { "▸" } else { " " };
        let text = format!("{marker} {} · {}", age(now, candidate.updated), candidate.last_line);
        let style =
            if position == index { Style::new().add_modifier(Modifier::REVERSED) } else { Style::new() };
        lines.push(Line::styled(fit(&text, area.width), style));
    }
    frame.render_widget(Paragraph::new(lines), area);
    push_rows(app, area, (offset, area.y + 2), candidates.len(), Hit::ListRow);
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
#[allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]
mod tests {

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn selection_wins_over_a_find_match() {
        use super::{Marks, range_style, row_spans, style_of};
        use crate::doc::{DocSpan, SpanStyle};
        use crate::review::Kind;
        use crate::wrap::VisualLine;
        use ratatui::style::{Color, Modifier, Style};
        let row = VisualLine {
            doc_line: 0,
            spans: vec![DocSpan { text: "abcdef".to_owned(), style: SpanStyle::default(), source: None }],
            first: true,
        };
        let marks = Marks {
            kinds: vec![Some(Kind::Comment); 6],
            selected: vec![false, true, false, false, false, false],
        };
        let spans = row_spans(&row, &[0..4], None, &marks);
        let style_of_char = |wanted: usize| {
            let mut at = 0;
            for span in &spans {
                let len = span.content.chars().count();
                if wanted < at + len {
                    return span.style;
                }
                at += len;
            }
            unreachable!()
        };
        let noted = style_of(SpanStyle::default()).patch(range_style(Kind::Comment));
        let highlight = Style::new().fg(Color::Black).bg(Color::Yellow);
        assert_eq!(style_of_char(1), noted.add_modifier(Modifier::REVERSED));
        assert_eq!(style_of_char(0), highlight);
        assert_eq!(style_of_char(2), highlight);
        assert_eq!(style_of_char(4), noted);
    }
    use jiff::tz::{TimeZone, offset};

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::super::hits::Hit;
    use super::super::{AgentState, App, Screen};
    use super::{draw, short_time};
    use crate::config::Config;
    use crate::replies::Reply;
    use crate::review::Kind;
    use crate::safety::Safety;

    #[test]
    fn a_search_on_a_long_line_scans_the_line_once_for_each_frame() {
        let reply = Reply { id: "r".to_owned(), text: "abc ".repeat(25_000), at: None };
        let state = AgentState { agent: None, safety: Safety::Clear { state: None } };
        let mut app = App::new(state, Screen::Viewer, vec![reply], &Config::default());
        app.query = Some("abc".to_owned());
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        super::MATCH_RUNS.with(|runs| runs.set(0));
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        let runs = super::MATCH_RUNS.with(std::cell::Cell::get);
        assert_eq!(runs, 1, "the line was joined and searched {runs} times in one frame");
    }

    #[test]
    fn a_dense_search_visits_each_match_about_once_for_each_frame() {
        let reply = Reply { id: "r".to_owned(), text: "ab ".repeat(30_000), at: None };
        let state = AgentState { agent: None, safety: Safety::Clear { state: None } };
        let mut app = App::new(state, Screen::Viewer, vec![reply], &Config::default());
        app.query = Some("a".to_owned());
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        super::MATCH_VISITS.with(|visits| visits.set(0));
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        let visits = super::MATCH_VISITS.with(std::cell::Cell::get);
        let rows = app.cache.get(&app.doc, 120).len();
        assert!(visits <= 30_000 + rows, "{visits} match visits for 30000 matches and {rows} rows");
    }

    #[test]
    fn doc_cells_exist_past_visual_row_65535() {
        let text = (0..70_000).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n\n");
        let reply = Reply { id: "r".to_owned(), text, at: None };
        let state = AgentState { agent: None, safety: Safety::Clear { state: None } };
        let mut app = App::new(state, Screen::Viewer, vec![reply], &Config::default());
        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        assert!(matches!(app.hits.at(2, 1), Some(Hit::DocCell { .. })));
    }

    #[test]
    fn each_visible_block_is_in_the_hit_map() {
        let reply = Reply { id: "r".to_owned(), text: "# T\n\none\ntwo\n\nthree\n".to_owned(), at: None };
        let state = AgentState { agent: None, safety: Safety::Clear { state: None } };
        let mut app = App::new(state, Screen::Viewer, vec![reply], &Config::default());
        // The view starts without a focus: `j` focuses the first block.
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        let screen = terminal.backend().buffer().clone();
        let row_of = |text: &str| {
            (0..24u16)
                .find(|&y| {
                    (0..80u16)
                        .map(|x| screen.cell((x, y)).map_or(" ", |c| c.symbol()))
                        .collect::<String>()
                        .contains(text)
                })
                .expect("row")
        };
        let blocks = app.doc.blocks.len();
        assert_eq!(blocks, 3);
        for (text, index) in [("T", 0), ("one", 1), ("three", 2)] {
            let y = row_of(text);
            assert_eq!(app.hits.at(0, y), Some(&Hit::Block(index)), "margin of {text}");
            assert!(matches!(app.hits.at(2, y), Some(Hit::DocCell { .. })), "text of {text}");
        }
        // The second row of the block with two lines.
        assert_eq!(app.hits.at(0, row_of("two")), Some(&Hit::Block(1)));
        // The blank rows between blocks are in no block. A focus alone opens no action strip.
        assert_eq!(app.hits.at(0, row_of("three") - 1), None);
        assert_eq!(app.hits.at(0, row_of("one") - 1), None);
    }

    /// The column and the row of the first cell where `text` starts on the screen. A wide character
    /// fills two cells, and the second cell has an empty symbol, so each byte maps back to its cell.
    fn find(screen: &ratatui::buffer::Buffer, text: &str) -> (u16, u16) {
        let area = screen.area;
        for y in 0..area.height {
            let mut line = String::new();
            let mut cells = Vec::new();
            for x in 0..area.width {
                let symbol = screen.cell((x, y)).map_or(" ", |c| c.symbol());
                cells.extend(std::iter::repeat_n(x, symbol.len()));
                line.push_str(symbol);
            }
            if let Some(&x) = line.find(text).and_then(|at| cells.get(at)) {
                return (x, y);
            }
        }
        panic!("no {text:?} on the screen");
    }

    fn drawn(text: &str, keys: &str) -> (App, ratatui::buffer::Buffer) {
        let reply = Reply { id: "r".to_owned(), text: text.to_owned(), at: None };
        let state = AgentState { agent: None, safety: Safety::Clear { state: None } };
        let mut app = App::new(state, Screen::Viewer, vec![reply], &Config::default());
        // The view starts without a focus: `j` focuses the first block, then come the `keys`.
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        for c in keys.chars() {
            let code = if c == '\n' { KeyCode::Enter } else { KeyCode::Char(c) };
            app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        let screen = terminal.backend().buffer().clone();
        (app, screen)
    }

    #[test]
    fn the_strip_items_are_in_the_hit_map() {
        let (app, screen) = drawn("# T\n\none\n\ntwo\n", "j\n");
        for (key, word, kind) in [
            ("o ", "looks good", Kind::LooksGood),
            ("c ", "comment", Kind::Comment),
            ("d ", "remove", Kind::Delete),
            ("s ", "replace", Kind::Replace),
        ] {
            let (end, y) = find(&screen, word);
            let last = end + u16::try_from(word.len()).expect("len") - 1;
            assert_eq!(app.hits.at(last, y), Some(&Hit::Action(kind)), "{word}");
            // The key starts the item: the icon is between the key and the word.
            let (x, _) = find(&screen, &format!("{key}{}", super::super::cards::icon(kind)));
            assert_eq!(app.hits.at(x, y), Some(&Hit::Action(kind)), "{word}");
            assert_ne!(app.hits.at(x - 1, y), Some(&Hit::Action(kind)), "{word}");
        }
    }

    #[test]
    fn a_focus_alone_puts_no_action_in_the_hit_map() {
        let (app, screen) = drawn("# T\n\none\n\ntwo\n", "j");
        let area = screen.area;
        for y in 0..area.height {
            for x in 0..area.width {
                let hit = app.hits.at(x, y);
                assert!(!matches!(hit, Some(Hit::Action(_) | Hit::Bar)), "{x},{y}: {hit:?}");
            }
        }
    }

    #[test]
    fn the_editor_has_save_and_cancel_in_the_hit_map() {
        let (app, screen) = drawn("# T\n\none\n\ntwo\n", "jc");
        let (x, y) = find(&screen, " save ");
        assert_eq!(app.hits.at(x, y), Some(&Hit::EditorSave));
        assert_eq!(app.hits.at(x + 5, y), Some(&Hit::EditorSave));
        let (x, y) = find(&screen, " cancel ");
        assert_eq!(app.hits.at(x, y), Some(&Hit::EditorCancel));
        assert_eq!(app.hits.at(x + 7, y), Some(&Hit::EditorCancel));
        assert_ne!(app.hits.at(x + 8, y), Some(&Hit::EditorCancel));
    }

    #[test]
    fn the_editor_box_clamps_into_the_area_when_it_fits_neither_below_nor_above() {
        let area = ratatui::layout::Rect::new(0, 0, 40, 10);
        assert_eq!(super::editor_top(area, 6, Some((3, 5))), 4, "below is 6 + 6 > 10, above is 3 < 6");
        assert_eq!(super::editor_top(area, 3, Some((1, 2))), 3, "below fits");
        assert_eq!(super::editor_top(area, 3, Some((6, 8))), 3, "above fits");
        assert_eq!(super::editor_top(area, 3, None), 7, "no range: the bottom of the area");
    }

    /// A document of 40 short blocks, scrolled to the bottom, with `keys` typed on the last block.
    fn last_block_with(keys: &str) -> (App, ratatui::buffer::Buffer) {
        let text = (0..40).map(|i| format!("p{i}")).collect::<Vec<_>>().join("\n\n");
        let (mut app, _) = drawn(&text, "");
        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
        for c in keys.chars() {
            let code = if c == '\n' { KeyCode::Enter } else { KeyCode::Char(c) };
            app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        }
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        let screen = terminal.backend().buffer().clone();
        (app, screen)
    }

    #[test]
    fn the_card_of_the_last_block_shows_at_the_bottom() {
        let (app, screen) = last_block_with("o");
        let (x, y) = find(&screen, "looks good");
        assert!(matches!(app.hits.at(x, y + 1), Some(Hit::Card(_))), "the moved card is clickable");
    }

    #[test]
    fn a_tall_card_of_the_last_block_shows_at_the_bottom() {
        let comment = "a long comment that wraps over several rows of the card in the margin of the screen";
        let (app, screen) = last_block_with(&format!("c{comment}\n"));
        let (x, y) = find(&screen, "comment");
        let body = (y + 1..24).find(|&row| app.hits.at(x, row) == Some(&Hit::Card(1)));
        assert!(body.is_some(), "the card is in the hit map");
        let (_, last) = find(&screen, "screen");
        assert!(last > y + 1, "the body takes more than one row");
    }

    #[test]
    fn each_card_shows_its_own_note_when_an_early_card_is_left_out() {
        // Ten notes, from the last block upward. The note `n{i}x` has the id `i + 1`.
        let keys = (0..10).fold(String::new(), |keys, i| keys + &format!("cn{i}x\nk"));
        let (app, screen) = last_block_with(&keys);
        let rows: Vec<String> = (0..24)
            .map(|y| (0..120).map(|x| screen.cell((x, y)).map_or(" ", |c| c.symbol())).collect())
            .collect();
        let mut shown = 0;
        for i in 0..10_u64 {
            let text = format!("n{i}x");
            let found = rows.iter().enumerate().find_map(|(y, row)| row.find(&text).map(|x| (x, y)));
            let Some((x, y)) = found else { continue };
            shown += 1;
            let (x, y) = (u16::try_from(x).expect("x"), u16::try_from(y).expect("y"));
            assert_eq!(app.hits.at(x, y), Some(&Hit::Card(i + 1)), "the card with {text}");
        }
        assert!(shown >= 5, "cards show: {shown}");
        assert!(shown < 10, "a card is left out: {shown}");
    }

    #[test]
    fn the_strip_goes_above_the_last_block_at_the_bottom() {
        let text = (0..40).map(|i| format!("p{i}")).collect::<Vec<_>>().join("\n\n");
        let (mut app, _) = drawn(&text, "");
        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        let screen = terminal.backend().buffer().clone();
        let (_, bar) = find(&screen, "comment");
        let (_, last) = find(&screen, "p39");
        assert_eq!(bar + 1, last, "the strip is on the row above the block");
    }

    #[test]
    fn the_strip_hides_when_another_part_has_the_focus_or_the_find_input_is_open() {
        for keys in ["\na", "\n/", "\np", "\n?"] {
            let (_, screen) = drawn("# T\n\none\n", keys);
            let text: String = (0..24u16)
                .map(|y| {
                    (0..80u16).map(|x| screen.cell((x, y)).map_or(" ", |c| c.symbol())).collect::<String>()
                })
                .collect();
            assert!(!text.contains("c comment"), "after {keys:?}");
        }
        let (_, screen) = drawn("# T\n\none\n", "\n");
        assert_eq!(find(&screen, "comment").1, 2, "the strip shows after enter on the block");
    }

    #[test]
    fn the_strip_alone_belongs_to_the_bar() {
        let reply = Reply { id: "r".to_owned(), text: "one\ntwo\nthree\n".to_owned(), at: None };
        let state = AgentState { agent: None, safety: Safety::Clear { state: None } };
        let mut app = App::new(state, Screen::Viewer, vec![reply], &Config::default())
            .with_root(std::path::PathBuf::from("/work"));
        app.open_file(crate::files::LoadedFile {
            relative: std::path::PathBuf::from("a.txt"),
            text: "one\ntwo\nthree\n".to_owned(),
            stamp: crate::files::Stamp { size: 14, modified: None },
        });
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        let screen = terminal.backend().buffer().clone();
        let (x, y) = find(&screen, "\u{2590}");
        let (end, _) = find(&screen, "\u{258c}");
        assert_eq!(y, find(&screen, "three").1 + 1, "below the block: it starts on the first row");
        assert_eq!(x, find(&screen, "one").0, "at the text column");
        assert_eq!(app.hits.at(x, y), Some(&Hit::Bar), "the left cap");
        assert_eq!(app.hits.at(end, y), Some(&Hit::Bar), "the right cap");
        assert_ne!(app.hits.at(0, y), Some(&Hit::Bar), "the margin of the row");
        assert_ne!(app.hits.at(end + 1, y), Some(&Hit::Bar), "the row after the strip");
    }

    /// A file viewer at 80 x 24 (one column) with a scanned tree, `a.txt` open with one note, and
    /// `keys` pressed after the first frame.
    fn one_column_with(keys: &str) -> (App, ratatui::buffer::Buffer) {
        let state = AgentState { agent: None, safety: Safety::Clear { state: None } };
        let mut app = App::new(state, Screen::Viewer, Vec::new(), &Config::default())
            .with_root(std::path::PathBuf::from("/work"));
        app.set_scan(crate::files::Scan {
            entries: vec![crate::files::Entry { path: "a.txt".into(), is_dir: false }],
            truncated: false,
        });
        app.open_file(crate::files::LoadedFile {
            relative: std::path::PathBuf::from("a.txt"),
            text: "one\ntwo\nthree\nfour\n".to_owned(),
            stamp: crate::files::Stamp { size: 19, modified: None },
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        for c in "jcnote\n".chars() {
            let code = if c == '\n' { KeyCode::Enter } else { KeyCode::Char(c) };
            app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        }
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        for c in keys.chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        let screen = terminal.backend().buffer().clone();
        (app, screen)
    }

    /// True when no cell of the body (rows 1 to 22) reaches the document below an overlay.
    fn no_doc_hit_in_the_body(app: &App) -> bool {
        (1..23u16).all(|y| {
            (0..80u16).all(|x| !matches!(app.hits.at(x, y), Some(Hit::Block(_) | Hit::DocCell { .. })))
        })
    }

    #[test]
    fn the_tree_over_the_document_covers_the_document_hits() {
        let (app, screen) = one_column_with("T");
        let (_, title) = find(&screen, "┌ Tree");
        assert_eq!(title, 1, "the tree covers the body");
        assert!(no_doc_hit_in_the_body(&app), "border, rows, and the empty rows below the last row");
        let (x, y) = find(&screen, "a.txt");
        assert_eq!(app.hits.at(x, y), Some(&Hit::TreeRow(0)));
        assert_eq!(app.hits.at(x, y + 3), Some(&Hit::BoxInside), "an empty row under the last tree row");
    }

    #[test]
    fn the_annotation_list_over_the_document_covers_the_document_hits() {
        let (app, screen) = one_column_with("a");
        let (_, title) = find(&screen, "┌ Annotations");
        assert_eq!(title, 1);
        assert!(no_doc_hit_in_the_body(&app));
        let (x, y) = find(&screen, "l.1  comment");
        assert_eq!(app.hits.at(x, y), Some(&Hit::ListRow(0)));
        assert_eq!(app.hits.at(x, y + 1), Some(&Hit::ListRow(0)), "the second line of the item");
    }

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
