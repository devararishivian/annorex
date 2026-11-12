//! The mouse: clicks, drags, double clicks, and the wheel, on the hit map of the last frame.

#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use std::path::Path;
use std::time::SystemTime;

use annorex::agent::Agent;
use annorex::config::Config;
use annorex::deliver::DeliveryMode;
use annorex::replies::{Candidate, Reply, SessionRef};
use annorex::review::Kind;
use annorex::safety::Safety;
use annorex::ui::{AgentState, App, Effect, Mode, Screen, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

const WIDE: (u16, u16) = (120, 20);
const NARROW: (u16, u16) = (80, 24);
const THREE: &str = "para one\n\npara two\n\npara three\n";

fn state() -> AgentState {
    AgentState { agent: Some(Agent::ClaudeCode), safety: Safety::Clear { state: Some("idle".to_owned()) } }
}

fn reply(id: &str, text: &str) -> Reply {
    Reply { id: id.to_owned(), text: text.to_owned(), at: Some("2026-11-09T10:30:30.000Z".to_owned()) }
}

fn viewer(text: &str) -> App {
    let mut app = App::new(state(), Screen::Viewer, vec![reply("r1", text)], &Config::default());
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    app
}

fn press(app: &mut App, keys: &str) {
    for c in keys.chars() {
        let code = if c == '\n' { KeyCode::Enter } else { KeyCode::Char(c) };
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }
}

fn frame(app: &mut App, (width, height): (u16, u16)) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    terminal.backend().buffer().clone()
}

/// The text of each row of a frame.
fn rows(buffer: &Buffer) -> Vec<String> {
    let area = buffer.area;
    (area.y..area.bottom())
        .map(|y| (area.x..area.right()).map(|x| buffer.cell((x, y)).map_or(" ", |c| c.symbol())).collect())
        .collect()
}

/// The screen cell of the first `text`, from the column `from`. Wide characters fill two cells, and the
/// second cell has an empty symbol, so the byte index maps back to the cell.
fn find_from(buffer: &Buffer, text: &str, from: u16) -> Option<(u16, u16)> {
    let area = buffer.area;
    for y in area.y..area.bottom() {
        let mut line = String::new();
        let mut cells = Vec::new();
        for x in from..area.right() {
            let symbol = buffer.cell((x, y)).map_or(" ", |c| c.symbol());
            let symbol = if symbol.is_empty() { "\u{0}" } else { symbol };
            for _ in 0..symbol.len() {
                cells.push(x);
            }
            line.push_str(symbol);
        }
        if let Some(index) = line.find(text) {
            return Some((cells[index], y));
        }
    }
    None
}

fn find(buffer: &Buffer, text: &str) -> (u16, u16) {
    find_from(buffer, text, 0)
        .unwrap_or_else(|| panic!("`{text}` is not on the screen:\n{}", rows(buffer).join("\n")))
}

fn event(kind: MouseEventKind, (column, row): (u16, u16)) -> MouseEvent {
    MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }
}

/// A press and a release of the left button at one cell. The result is the effect of the press.
fn click(app: &mut App, at: (u16, u16)) -> Effect {
    let effect = app.handle_mouse(event(MouseEventKind::Down(MouseButton::Left), at));
    app.handle_mouse(event(MouseEventKind::Up(MouseButton::Left), at));
    effect
}

fn wheel_down(app: &mut App, at: (u16, u16)) {
    app.handle_mouse(event(MouseEventKind::ScrollDown, at));
}

fn footer(buffer: &Buffer) -> String {
    rows(buffer).last().cloned().unwrap_or_default()
}

/// An app with a comment `first` on block 0 and a comment `third` on block 2. Block 2 has the focus.
/// A document of `count` paragraphs `para 0`, `para 1`, and so on.
fn paragraphs(count: usize) -> String {
    (0..count).map(|n| format!("para {n}\n\n")).collect::<Vec<_>>().concat()
}

fn two_notes() -> App {
    let mut app = viewer(THREE);
    press(&mut app, "cfirst\njjcthird\n");
    assert_eq!(app.review().expect("review").annotations.len(), 2);
    assert_eq!(app.focused_block(), Some(2));
    app
}

#[test]
fn click_on_a_block_focuses_it() {
    let mut app = viewer(THREE);
    let screen = frame(&mut app, NARROW);
    assert_eq!(app.focused_block(), Some(0));
    click(&mut app, find(&screen, "para three"));
    assert_eq!(app.mode(), Mode::Block(2));
    // A click in the left margin of a block also focuses it.
    let (_, y) = find(&screen, "para two");
    click(&mut app, (0, y));
    assert_eq!(app.mode(), Mode::Block(1));
}

#[test]
fn click_on_an_action_button_does_it() {
    let mut app = viewer(THREE);
    let screen = frame(&mut app, NARROW);
    assert_eq!(click(&mut app, find(&screen, "looks good]")), Effect::SaveDraft);
    let review = app.review().expect("review");
    assert_eq!(review.annotations.len(), 1);
    assert_eq!(review.annotations[0].kind, Kind::LooksGood);
    assert!(app.editor().is_none(), "looks good saves at once");
    let screen = frame(&mut app, NARROW);
    click(&mut app, find(&screen, "comment]"));
    assert_eq!(app.editor().expect("editor").kind, Kind::Comment);
}

#[test]
fn drag_selects_text_and_shows_the_bar() {
    let mut app = viewer("alpha beta gamma\n");
    let screen = frame(&mut app, NARROW);
    let (x, y) = find(&screen, "beta");
    app.handle_mouse(event(MouseEventKind::Down(MouseButton::Left), (x, y)));
    app.handle_mouse(event(MouseEventKind::Drag(MouseButton::Left), (x + 2, y)));
    app.handle_mouse(event(MouseEventKind::Drag(MouseButton::Left), (x + 3, y)));
    app.handle_mouse(event(MouseEventKind::Up(MouseButton::Left), (x + 3, y)));
    assert_eq!(app.mode(), Mode::Text);
    let screen = frame(&mut app, NARROW);
    assert!(rows(&screen).iter().any(|row| row.contains("looks good]")), "the bar shows after the release");
    press(&mut app, "c");
    assert_eq!(app.editor().expect("editor").range, 6..10, "the drag selected `beta`");
}

#[test]
fn drag_at_the_bottom_row_scrolls() {
    let text = paragraphs(60);
    let mut app = viewer(&text);
    let screen = frame(&mut app, NARROW);
    let (x, y) = find(&screen, "para 0");
    app.handle_mouse(event(MouseEventKind::Down(MouseButton::Left), (x, y)));
    // The last row of the document area is above the footer.
    let bottom = NARROW.1 - 2;
    for _ in 0..4 {
        app.handle_mouse(event(MouseEventKind::Drag(MouseButton::Left), (x, bottom)));
    }
    app.handle_mouse(event(MouseEventKind::Up(MouseButton::Left), (x, bottom)));
    let screen = frame(&mut app, NARROW);
    assert!(!rows(&screen)[1].contains("para 0"), "the view moved down:\n{}", rows(&screen).join("\n"));
    assert_eq!(app.mode(), Mode::Text);
}

#[test]
fn double_click_selects_the_word() {
    let mut app = viewer("alpha beta gamma\n");
    let screen = frame(&mut app, NARROW);
    let (x, y) = find(&screen, "gamma");
    click(&mut app, (x + 1, y));
    click(&mut app, (x + 1, y));
    assert_eq!(app.mode(), Mode::Text);
    press(&mut app, "c");
    assert_eq!(app.editor().expect("editor").range, 11..16, "the double click selected `gamma`");
}

#[test]
fn wheel_scrolls_the_part_under_the_pointer_without_moving_the_focus() {
    let text = paragraphs(40);
    let mut app = viewer(&text);
    let screen = frame(&mut app, NARROW);
    assert!(rows(&screen)[1].contains("para 0"));
    wheel_down(&mut app, find(&screen, "para 1"));
    let screen = frame(&mut app, NARROW);
    assert!(!rows(&screen)[1].contains("para 0"), "the document scrolled:\n{}", rows(&screen).join("\n"));
    assert_eq!(app.mode(), Mode::Block(0), "the focus stays");

    // The tree on the left.
    let dir = tempfile::tempdir().expect("dir");
    for n in 0..40 {
        std::fs::write(dir.path().join(format!("f{n:02}.txt")), "text\n").expect("write");
    }
    let mut app = files_app(dir.path());
    let screen = frame(&mut app, WIDE);
    click(&mut app, find(&screen, "f00.txt"));
    assert_eq!(app.shown_file(), Some(Path::new("f00.txt")));
    let screen = frame(&mut app, WIDE);
    let first = find(&screen, "f00.txt");
    wheel_down(&mut app, first);
    let screen = frame(&mut app, WIDE);
    assert!(find_from(&screen, "f00.txt", 0).is_none_or(|(x, _)| x > first.0), "the tree scrolled");
    assert_eq!(find(&screen, "f03.txt").1, first.1, "three rows");
    assert!(footer(&screen).contains("block"), "the document keeps the focus: {}", footer(&screen));
}

fn files_app(root: &Path) -> App {
    let mut app =
        App::new(state(), Screen::Viewer, Vec::new(), &Config::default()).with_root(root.to_path_buf());
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    app.set_scan(annorex::files::scan(root));
    app
}

#[test]
fn click_on_send_is_the_same_as_shift_s() {
    let mut app = two_notes();
    let screen = frame(&mut app, WIDE);
    let effect = click(&mut app, find(&screen, "[ Send"));
    assert_eq!(effect, Effect::Send(DeliveryMode::Send));
    let mut keys = two_notes();
    assert_eq!(keys.handle_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE)), effect);
    // With nothing to send, the click gives the same message as the key.
    let mut empty = viewer(THREE);
    let screen = frame(&mut empty, WIDE);
    assert_eq!(click(&mut empty, find(&screen, "[ Send")), Effect::None);
    assert_eq!(empty.status(), Some("Nothing to send: no annotations yet."));
}

#[test]
fn click_on_a_tree_row_opens_the_file() {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::write(dir.path().join("a.txt"), "alpha\n").expect("write");
    let mut app = files_app(dir.path());
    let screen = frame(&mut app, WIDE);
    assert_eq!(click(&mut app, find(&screen, "a.txt")), Effect::FileOpened);
    assert_eq!(app.shown_file(), Some(Path::new("a.txt")));
}

#[test]
fn click_on_a_folder_toggles_it() {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::create_dir(dir.path().join("src")).expect("dir");
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").expect("write");
    let mut app = files_app(dir.path());
    let screen = frame(&mut app, WIDE);
    let folder = find(&screen, "src/");
    let shown = |app: &mut App| rows(&frame(app, WIDE)).iter().any(|row| row.contains("main.rs"));
    let before = shown(&mut app);
    click(&mut app, folder);
    assert_ne!(shown(&mut app), before, "the click opened or closed the folder");
    click(&mut app, folder);
    assert_eq!(shown(&mut app), before, "a second click undid it");
}

#[test]
fn click_on_a_reply_row_opens_it() {
    let replies =
        vec![reply("r1", "first reply\n"), reply("r2", "second reply\n"), reply("r3", "third reply\n")];
    let mut app = App::new(state(), Screen::Viewer, replies, &Config::default());
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    press(&mut app, "p");
    let screen = frame(&mut app, NARROW);
    click(&mut app, find(&screen, "second reply"));
    assert_eq!(app.shown_reply_id(), Some("r2"));
    let screen = frame(&mut app, NARROW);
    assert!(!rows(&screen).iter().any(|row| row.contains("Replies")), "the list closed");

    // A row of the session picker chooses the session.
    let candidate = |name: &str| Candidate {
        session: SessionRef::Pi(format!("/{name}.jsonl").into()),
        updated: SystemTime::now(),
        last_line: name.to_owned(),
    };
    let mut picker = App::new(state(), Screen::Viewer, Vec::new(), &Config::default());
    picker.set_picker(vec![candidate("session-one"), candidate("session-two")]);
    let screen = frame(&mut picker, NARROW);
    assert_eq!(click(&mut picker, find(&screen, "session-two")), Effect::PickSession(1));
}

#[test]
fn click_on_a_card_focuses_its_note() {
    let mut app = two_notes();
    let screen = frame(&mut app, WIDE);
    click(&mut app, find_from(&screen, "first", 86).expect("card"));
    assert_eq!(app.mode(), Mode::Block(0), "the block of the note has the focus");
    let screen = frame(&mut app, WIDE);
    let (_, edit_row) = find(&screen, "e edit");
    let (_, card_row) = find_from(&screen, "first", 86).expect("card");
    assert!(edit_row > card_row && edit_row <= card_row + 2, "the focused card shows its buttons");

    // A row of the annotation list does the same.
    let mut app = two_notes();
    press(&mut app, "a");
    let screen = frame(&mut app, NARROW);
    click(&mut app, find(&screen, "first"));
    assert_eq!(app.mode(), Mode::Block(0));
    assert!(footer(&frame(&mut app, NARROW)).contains("block 1/3"), "the document has the focus again");
}

#[test]
fn click_on_card_edit_and_remove() {
    let mut app = two_notes();
    let screen = frame(&mut app, WIDE);
    click(&mut app, find(&screen, "e edit"));
    let editor = app.editor().expect("editor");
    assert_eq!(editor.text, "third", "edit opens the note");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.editor().is_none());
    let screen = frame(&mut app, WIDE);
    click(&mut app, find(&screen, "x remove"));
    let screen = frame(&mut app, WIDE);
    assert!(footer(&screen).contains("Delete annotation 2? y/n"), "{}", footer(&screen));
    press(&mut app, "y");
    assert_eq!(app.review().expect("review").annotations.len(), 1);
}

#[test]
fn click_on_a_stale_card_does_nothing() {
    let mut app = two_notes();
    let screen = frame(&mut app, WIDE);
    let remove = find(&screen, "x remove");
    let edit = find(&screen, "e edit");
    // Remove the note with the keys. No frame is drawn after it.
    press(&mut app, "xy");
    assert_eq!(app.review().expect("review").annotations.len(), 1);
    click(&mut app, remove);
    click(&mut app, edit);
    assert!(app.editor().is_none());
    let screen = frame(&mut app, WIDE);
    assert!(!footer(&screen).contains("y/n"), "no question for a removed note: {}", footer(&screen));
}

#[test]
fn click_on_a_footer_hint_does_the_action() {
    let mut app = viewer(THREE);
    let screen = frame(&mut app, WIDE);
    click(&mut app, find(&screen, "p list"));
    let screen = frame(&mut app, WIDE);
    assert!(rows(&screen).iter().any(|row| row.contains("Replies")), "the reply list opened");
    let mut app = viewer(THREE);
    let screen = frame(&mut app, WIDE);
    click(&mut app, find(&screen, "? help"));
    assert!(app.help_open);
}

#[test]
fn click_outside_the_editor_keeps_it_open_with_its_text() {
    let mut app = viewer(THREE);
    press(&mut app, "chi");
    let screen = frame(&mut app, NARROW);
    click(&mut app, find(&screen, "para one"));
    click(&mut app, (0, 0));
    click(&mut app, (0, NARROW.1 - 1));
    assert_eq!(app.editor().expect("editor").text, "hi");
}

#[test]
fn click_on_save_and_cancel() {
    let mut app = viewer(THREE);
    press(&mut app, "chi");
    let screen = frame(&mut app, NARROW);
    click(&mut app, find(&screen, " save "));
    assert!(app.editor().is_none());
    assert_eq!(app.review().expect("review").annotations[0].note, "hi");

    let mut app = viewer(THREE);
    press(&mut app, "chi");
    let screen = frame(&mut app, NARROW);
    click(&mut app, find(&screen, " cancel "));
    assert!(app.editor().is_none());
    assert!(app.review().is_none_or(|review| review.annotations.is_empty()));
}

#[test]
fn mouse_with_help_open_closes_help() {
    let mut app = viewer(THREE);
    press(&mut app, "?");
    let screen = frame(&mut app, NARROW);
    click(&mut app, find(&screen, "Keys"));
    assert!(!app.help_open);
    press(&mut app, "?");
    frame(&mut app, NARROW);
    app.handle_mouse(event(MouseEventKind::Down(MouseButton::Right), (1, 1)));
    assert!(!app.help_open, "any button closes the help");
}

#[test]
fn confirm_question_ignores_clicks() {
    let mut app = two_notes();
    press(&mut app, "x");
    let screen = frame(&mut app, WIDE);
    assert!(footer(&screen).contains("y/n"));
    click(&mut app, find(&screen, "para one"));
    click(&mut app, find(&screen, "[ Send"));
    assert_eq!(app.mode(), Mode::Block(2));
    let screen = frame(&mut app, WIDE);
    assert!(footer(&screen).contains("y/n"), "the question stays: {}", footer(&screen));
}

#[test]
fn click_outside_a_box_closes_it() {
    let mut app = viewer(THREE);
    press(&mut app, "p");
    let screen = frame(&mut app, WIDE);
    assert!(rows(&screen).iter().any(|row| row.contains("Replies")));
    // The footer hint for help is outside the box: the click closes the box and does not open help.
    click(&mut app, find(&screen, "? help"));
    assert!(!app.help_open);
    let screen = frame(&mut app, WIDE);
    assert!(!rows(&screen).iter().any(|row| row.contains("Replies")), "the box closed");
    // A click on the header send button is outside too.
    press(&mut app, "p");
    let screen = frame(&mut app, WIDE);
    assert_eq!(click(&mut app, find(&screen, "[ Send")), Effect::None);
    let screen = frame(&mut app, WIDE);
    assert!(!rows(&screen).iter().any(|row| row.contains("Replies")), "the box closed");
}

#[test]
fn click_on_a_row_of_the_numbered_pick_chooses_the_note() {
    let mut app = viewer(THREE);
    press(&mut app, "cfirst\ncsecond\ne");
    let screen = frame(&mut app, NARROW);
    assert!(rows(&screen).iter().any(|row| row.contains("pick")), "the pick is open");
    click(&mut app, find(&screen, "second"));
    assert_eq!(app.editor().expect("editor").text, "second");
}
