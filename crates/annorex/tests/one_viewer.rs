//! One Annorex with replies and files: one list, one export, one send.

#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use std::path::PathBuf;

use annorex::agent::Agent;
use annorex::config::Config;
use annorex::files::{LoadedFile, Stamp};
use annorex::replies::Reply;
use annorex::replies::{Candidate, SessionRef};
use annorex::safety::Safety;
use annorex::ui::{AgentState, App, Effect, ReplySource, Screen, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

fn state() -> AgentState {
    AgentState { agent: Some(Agent::ClaudeCode), safety: Safety::Clear { state: Some("idle".to_owned()) } }
}

fn reply(id: &str, text: &str) -> Reply {
    Reply { id: id.to_owned(), text: text.to_owned(), at: Some("2026-11-09T10:30:30.000Z".to_owned()) }
}

fn loaded(relative: &str, text: &str) -> LoadedFile {
    LoadedFile {
        relative: PathBuf::from(relative),
        text: text.to_owned(),
        stamp: Stamp { size: text.len() as u64, modified: None },
    }
}

/// Two replies (`r1` is the newest) and the root `/work`. No file is open.
fn both() -> App {
    let replies = vec![reply("r1", "alpha beta gamma\n"), reply("r2", "older reply text\n")];
    let mut app =
        App::new(state(), Screen::Viewer, replies, &Config::default()).with_root(PathBuf::from("/work"));
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    app
}

fn press_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Effect {
    app.handle_key(KeyEvent::new(code, modifiers))
}

fn press(app: &mut App, keys: &str) {
    for c in keys.chars() {
        let code = if c == '\n' { KeyCode::Enter } else { KeyCode::Char(c) };
        press_key(app, code, KeyModifiers::NONE);
    }
}

/// A note on the newest reply, then a note on the file `a.md`.
fn noted() -> App {
    let mut app = both();
    press(&mut app, "cnote reply\n");
    app.open_file(loaded("a.md", "# Title\n\nalpha text\n"));
    press(&mut app, "cnote file\n");
    app
}

#[test]
fn a_send_contains_the_notes_of_a_reply_and_of_a_file() {
    let app = noted();
    let export = app.export();
    let reply = export.find("# Annotations on last reply").expect("reply section");
    let file = export.find("# Annotations on a.md").expect("file section");
    assert!(reply < file, "replies come before files:\n{export}");
    assert!(export.contains("> note reply") && export.contains("> note file"), "{export}");
}

#[test]
fn an_older_reply_is_named_with_its_time() {
    let mut app = both();
    press_key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
    press(&mut app, "cnote old\n");
    assert!(app.export().contains("# Annotations on earlier reply (10:30)"), "{}", app.export());
}

#[test]
fn finish_send_marks_the_reply_and_the_file_as_sent() {
    let mut app = noted();
    app.finish_send("2026-11-12T10:00:00Z");
    assert_eq!(app.export(), "");
    assert!(app.export_all().contains("# Annotations on a.md"));
    assert!(app.export_all().contains("# Annotations on last reply"));
    assert_eq!(app.shown_sent(), Some("2026-11-12T10:00:00Z"));
}

#[test]
fn the_quit_question_counts_a_reply_and_a_file_with_unsent_notes() {
    let mut app = noted();
    assert_eq!(app.unsent_file_count(), 2);
    assert_eq!(press_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE), Effect::None);
    app.finish_send("2026-11-12T10:00:00Z");
    assert_eq!(app.unsent_file_count(), 0);
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(press_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE), Effect::Quit);
}

#[test]
fn a_reply_can_be_shown_next_to_a_file() {
    let mut app = noted();
    assert_eq!(app.shown_file(), Some(std::path::Path::new("a.md")));
    press(&mut app, "[");
    assert_eq!(app.shown_file(), None);
    press(&mut app, "]");
    assert_eq!(app.shown_file(), Some(std::path::Path::new("a.md")));
}

#[test]
fn set_replies_keeps_open_files_and_their_notes() {
    let mut app = noted();
    app.set_replies(vec![reply("r9", "newest\n"), reply("r1", "alpha beta gamma\n")]);
    assert!(app.export().contains("> note file"), "{}", app.export());
    assert!(app.export().contains("> note reply"), "{}", app.export());
    assert_eq!(app.shown_file(), Some(std::path::Path::new("a.md")));
}

#[test]
fn the_draft_of_each_document_has_its_own_session() {
    let app = noted();
    let drafts = app.drafts(Agent::ClaudeCode, |id| {
        Some(if id.starts_with("file:") { "files:/work".to_owned() } else { "s1".to_owned() })
    });
    let sessions: Vec<(&str, &str)> =
        drafts.iter().map(|d| (d.key.document.as_str(), d.key.session.as_str())).collect();
    assert_eq!(sessions, vec![("r1", "s1"), ("file:a.md", "files:/work")]);
}

#[test]
fn discard_removes_only_the_shown_document() {
    let mut app = noted();
    press(&mut app, "X");
    press(&mut app, "y");
    assert_eq!(app.take_discarded(), vec!["file:a.md".to_owned()]);
    assert!(app.export().contains("> note reply"), "{}", app.export());
}

fn render(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let mut lines = Vec::new();
    for y in 0..height {
        let mut line = String::new();
        let mut x = 0;
        while x < width {
            let symbol = buffer.cell((x, y)).map_or(" ", |cell| cell.symbol());
            line.push_str(symbol);
            x += u16::try_from(symbol.width().max(1)).unwrap_or(1);
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}

/// No reply is loaded. The work directory is `/work`.
fn files_only() -> App {
    let mut app =
        App::new(state(), Screen::Viewer, Vec::new(), &Config::default()).with_root(PathBuf::from("/work"));
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    app
}

fn candidate(name: &str) -> Candidate {
    Candidate {
        session: SessionRef::Pi(PathBuf::from(format!("/{name}.jsonl"))),
        updated: std::time::SystemTime::now(),
        last_line: name.to_owned(),
    }
}

#[test]
fn a_missing_session_shows_its_reason_in_the_reply_list_and_the_files_still_open() {
    let mut app = files_only();
    app.set_reply_source(ReplySource::Missing("no reply yet".to_owned()));
    assert!(matches!(app.screen, Screen::Viewer), "the files must not be blocked");
    app.open_file(loaded("a.md", "# Title\n"));
    assert_eq!(app.shown_file(), Some(std::path::Path::new("a.md")));
    press(&mut app, "p");
    let screen = render(&mut app, 80, 24);
    assert!(screen.contains("no reply yet"), "{screen}");
    assert!(screen.contains("a.md"), "the open file is still listed:\n{screen}");
}

#[test]
fn the_picker_lists_the_sessions_in_the_reply_panel_and_enter_loads_one() {
    let mut app = files_only();
    app.set_reply_source(ReplySource::Pick(vec![candidate("first"), candidate("second")]));
    assert!(matches!(app.screen, Screen::Viewer));
    // With no document on the screen, the reply list opens by itself.
    let screen = render(&mut app, 80, 24);
    assert!(screen.contains("first") && screen.contains("second"), "{screen}");
    press(&mut app, "j");
    assert_eq!(press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE), Effect::PickSession(1));
}

#[test]
fn r_looks_for_the_reply_session_again_also_with_a_file_on_the_screen() {
    let mut app = files_only();
    app.set_reply_source(ReplySource::Missing("no reply yet".to_owned()));
    app.open_file(loaded("a.md", "# Title\n"));
    assert_eq!(press_key(&mut app, KeyCode::Char('r'), KeyModifiers::NONE), Effect::Reload);
}

#[test]
fn new_replies_make_the_source_ready() {
    let mut app = files_only();
    app.set_reply_source(ReplySource::Missing("no reply yet".to_owned()));
    app.set_replies(vec![reply("r1", "hello\n")]);
    assert_eq!(app.reply_source(), &ReplySource::Ready);
}

#[test]
fn the_header_names_the_position_among_replies_or_among_files() {
    let mut app = noted();
    let header = render(&mut app, 80, 24).lines().next().expect("header").to_owned();
    assert!(header.contains("file 1/1"), "{header}");
    press(&mut app, "[");
    press(&mut app, "[");
    let header = render(&mut app, 80, 24).lines().next().expect("header").to_owned();
    assert!(header.contains("reply 1/2"), "{header}");
}

#[test]
fn the_questions_name_the_document_on_the_screen() {
    let mut app = noted();
    press(&mut app, "X");
    assert!(render(&mut app, 100, 24).contains("Discard the notes of this file? y/n"));
    press(&mut app, "n");
    press(&mut app, "[");
    press(&mut app, "[");
    press(&mut app, "X");
    assert!(render(&mut app, 100, 24).contains("Discard the notes of this reply? y/n"));
    press(&mut app, "n");
    press_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
    assert!(render(&mut app, 120, 24).contains("Unsent notes in 2 documents."));
}
