//! Reviewing many files: the open files, the combined export, and the quit question.

#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use std::path::PathBuf;

use annorex::agent::Agent;
use annorex::config::Config;
use annorex::files::{LoadedFile, Stamp};
use annorex::safety::Safety;
use annorex::ui::{AgentState, App, Effect, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

fn state() -> AgentState {
    AgentState { agent: Some(Agent::ClaudeCode), safety: Safety::Clear { state: Some("idle".to_owned()) } }
}

fn loaded(relative: &str, text: &str) -> LoadedFile {
    LoadedFile {
        relative: PathBuf::from(relative),
        text: text.to_owned(),
        stamp: Stamp { size: text.len() as u64, modified: None },
    }
}

fn files_app() -> App {
    let mut app = App::new_files(state(), PathBuf::from("/work"), &Config::default());
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    app
}

/// An app with `b.txt` and then `a.md` open. `a.md` is on the screen.
fn two_files() -> App {
    let mut app = files_app();
    app.open_file(loaded("b.txt", "beta line\nsecond line\n"));
    app.open_file(loaded("a.md", "# Title\n\nalpha text\n"));
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

#[test]
fn export_joins_files_in_open_order() {
    let mut app = two_files();
    press(&mut app, "cnote a\n"); // a.md is on the screen: annotate it first
    press(&mut app, "[");
    press(&mut app, "cnote b\n");
    let export = app.export();
    let b = export.find("# Annotations on b.txt").expect("b.txt section");
    let a = export.find("# Annotations on a.md").expect("a.md section");
    assert!(b < a, "the sections follow the order of opening:\n{export}");
    assert!(export.contains("> note a") && export.contains("> note b"), "{export}");
    assert!(export.contains("## Annotation 1 (line 1)"), "line numbers come from each file:\n{export}");
    let mut with_note = two_files();
    press(&mut with_note, "cnote a\n");
    press(&mut with_note, "M");
    press(&mut with_note, "Please fix all.");
    assert_eq!(press_key(&mut with_note, KeyCode::Enter, KeyModifiers::NONE), Effect::SendWithNote);
    assert!(
        with_note.export().starts_with("Please fix all.\n\n# Annotations on a.md"),
        "{}",
        with_note.export()
    );
}

#[test]
fn each_file_keeps_its_own_review() {
    let mut app = two_files();
    press(&mut app, "[");
    press(&mut app, "cnote b\n");
    assert_eq!(app.review().expect("review").annotations.len(), 1);
    press(&mut app, "]");
    assert!(app.review().is_none_or(annorex::review::Review::is_empty), "a.md has no note");
    press(&mut app, "[");
    assert_eq!(app.review().expect("review").annotations[0].note, "note b");
}

#[test]
fn brackets_select_the_previous_and_next_file() {
    let mut app = two_files();
    assert_eq!(app.shown_reply_id(), Some("file:a.md"));
    press(&mut app, "[");
    assert_eq!(app.shown_reply_id(), Some("file:b.txt"));
    press(&mut app, "[");
    assert_eq!(app.shown_reply_id(), Some("file:b.txt"), "no file before the first");
    press(&mut app, "]]");
    assert_eq!(app.shown_reply_id(), Some("file:a.md"), "no file after the last");
    app.open_file(loaded("b.txt", "beta line\nsecond line\n"));
    assert_eq!(app.shown_reply_id(), Some("file:b.txt"), "opening an open file selects it");
    assert_eq!(app.reply_ids().len(), 2, "no second copy");
}

#[test]
fn list_marks_unsent_files() {
    let mut app = two_files();
    press(&mut app, "cnote a\n");
    let screen = render(&mut app, 120, 20);
    assert!(screen.lines().next().expect("header").contains("unsent 2"), "{screen}");
    press(&mut app, "p");
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("Files") && screen.contains("b.txt") && screen.contains("a.md"), "{screen}");
    let marked: Vec<&str> = screen.lines().filter(|line| line.contains('●')).collect();
    assert_eq!(marked.len(), 1, "{screen}");
    assert!(marked[0].contains("a.md"), "{marked:?}");
}

#[test]
fn closing_a_file_with_unsent_notes_asks() {
    let mut app = two_files();
    press(&mut app, "cnote a\n");
    press(&mut app, "W");
    assert!(render(&mut app, 120, 12).contains("Close a.md? It has unsent notes. y/n"));
    press(&mut app, "n");
    assert_eq!(app.reply_ids().len(), 2);
    press(&mut app, "Wy");
    assert_eq!(app.reply_ids(), ["file:b.txt"]);
    assert_eq!(app.shown_reply_id(), Some("file:b.txt"));
    assert!(!app.has_review("file:a.md"), "the notes of a closed file come back from the draft on disk");
    press(&mut app, "W");
    assert!(app.reply_ids().is_empty(), "a file without notes closes at once");
    assert_eq!(app.shown_reply_id(), None);
    assert_eq!(app.export(), "");
}

#[test]
fn plain_file_selection_gives_exact_ranges() {
    let mut app = files_app();
    let text = "fn main() {\n\tprintln!(\"héllo\");\n}\n";
    app.open_file(loaded("main.rs", text));
    press(&mut app, "jl");
    press(&mut app, "vllllllcx\n");
    press(&mut app, "g");
    press(&mut app, "jl");
    press(&mut app, &"l".repeat(10));
    press(&mut app, "vllllcy\n");
    let annotations = &app.review().expect("review").annotations;
    assert_eq!(annotations[0].quote, "println");
    assert_eq!(annotations[1].quote, "héllo");
    let start = text.find("héllo").expect("héllo");
    assert_eq!(annotations[1].range, start..start + "héllo".len());
}

#[test]
fn send_marks_all_files_sent() {
    let mut app = two_files();
    press(&mut app, "cnote a\n");
    press(&mut app, "[");
    press(&mut app, "cnote b\n");
    assert_ne!(app.export(), "");
    app.finish_send("2026-11-09T12:00:00Z".to_owned());
    assert!(app.export().is_empty(), "both files were sent");
    assert_eq!(app.unsent_replies(), Vec::<usize>::new());
    assert!(!app.export_all().is_empty(), "copy still has all notes");
    assert!(render(&mut app, 120, 12).contains("[Sent"));
}

#[test]
fn quit_question_counts_files() {
    let mut app = two_files();
    press(&mut app, "cnote a\n");
    press(&mut app, "[");
    press(&mut app, "cnote b\n");
    press(&mut app, "q");
    let screen = render(&mut app, 120, 12);
    assert!(
        screen.contains("Unsent notes in 2 files. s send · k keep the drafts · d discard · esc back"),
        "{screen}"
    );
    assert_eq!(press_key(&mut app, KeyCode::Char('d'), KeyModifiers::NONE), Effect::Discard);
    assert!(app.export().is_empty() && app.export_all().is_empty(), "d discards the notes of all files");
}

#[test]
fn snapshot_files_side_120x40() {
    let mut app = two_files();
    press(&mut app, "cnote a\n[cnote b\n");
    press(&mut app, "p");
    insta::assert_snapshot!(render(&mut app, 120, 40));
}

#[test]
fn snapshot_files_bottom_80x24() {
    let mut app = two_files();
    press(&mut app, "cnote a\n");
    press(&mut app, "p");
    insta::assert_snapshot!(render(&mut app, 80, 24));
}
