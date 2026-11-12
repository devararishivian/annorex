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

#[test]
fn opening_a_file_gives_the_keys_back_to_the_document_when_a_session_choice_waits() {
    let mut app = files_only();
    app.set_reply_source(ReplySource::Pick(vec![candidate("first"), candidate("second")]));
    app.open_file(loaded("a.md", "# Title\n\nalpha text\n"));
    press(&mut app, "cnote\n");
    assert!(app.export().contains("> note"), "the keys must reach the file:\n{}", app.export());
}

#[test]
fn the_send_button_counts_only_unsent_notes_of_all_documents() {
    let mut app = noted();
    app.finish_send("2026-11-12T10:00:00Z");
    press(&mut app, "cnote two\n");
    let screen = render(&mut app, 100, 24);
    assert!(screen.contains("[ Send 1 "), "one new note, not three:\n{screen}");
}

#[test]
fn a_new_reply_does_not_move_the_screen_off_an_open_file() {
    let mut app = files_only();
    app.set_reply_source(ReplySource::Missing("no reply yet".to_owned()));
    app.open_file(loaded("a.md", "# Title\n\nalpha text\n"));
    app.set_replies(vec![reply("r1", "hello\n")]);
    assert_eq!(app.shown_file(), Some(std::path::Path::new("a.md")));
}

const NEW_REPLY: &str = "The agent has a new reply. Press r to read it.";

fn screen_with_banner(app: &mut App) -> String {
    render(app, 200, 24)
}

#[test]
fn a_new_newest_reply_raises_the_banner() {
    let mut app = both();
    app.reply_polled("r9");
    assert!(screen_with_banner(&mut app).contains(NEW_REPLY));
}

#[test]
fn the_reply_the_user_already_sees_raises_no_banner() {
    let mut app = both();
    app.reply_polled("r1");
    assert!(!screen_with_banner(&mut app).contains(NEW_REPLY));
}

#[test]
fn reading_the_replies_again_clears_the_banner() {
    let mut app = both();
    app.reply_polled("r9");
    app.set_replies(vec![reply("r9", "new\n"), reply("r1", "alpha beta gamma\n")]);
    assert!(!screen_with_banner(&mut app).contains(NEW_REPLY));
}

#[test]
fn the_first_reply_of_an_empty_viewer_raises_the_banner() {
    let mut app = files_only();
    app.set_reply_source(ReplySource::Missing("no reply yet".to_owned()));
    app.reply_polled("r1");
    assert!(screen_with_banner(&mut app).contains(NEW_REPLY));
}

#[test]
fn the_reply_banner_shows_beside_the_banner_of_files() {
    let mut app = both();
    app.changed_list(vec!["a.rs".into()]);
    app.changed_list(vec!["b.rs".into(), "a.rs".into()]);
    app.reply_polled("r9");
    // Below 100 columns the tree is not docked, so the banner about its files stays.
    let screen = render(&mut app, 99, 24);
    assert!(screen.contains(NEW_REPLY) && screen.contains("The agent changed 1 more file"), "{screen}");
}

#[test]
fn a_poll_of_the_viewer_delivers_the_id_from_the_thread() {
    let mut app = both();
    let mut sent = false;
    app.start_reply_poll(move || {
        if sent {
            return None;
        }
        sent = true;
        Some("r9".to_owned())
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !screen_with_banner(&mut app).contains(NEW_REPLY) {
        app.poll_background();
        assert!(std::time::Instant::now() < deadline, "the banner never came");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn question_mark_shows_every_key_and_any_key_closes_the_help() {
    let mut app = both();
    press(&mut app, "?");
    let screen = render(&mut app, 100, 40);
    for text in
        [" Keys ", "S", "send the notes", "c", "comment", "ctrl+p", "find a file", "q", "close the viewer"]
    {
        assert!(screen.contains(text), "missing {text:?}:\n{screen}");
    }
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    let screen = render(&mut app, 100, 40);
    assert!(!screen.contains(" Keys "), "{screen}");
}

#[test]
fn the_key_that_closes_the_help_does_nothing_else() {
    let mut app = both();
    press(&mut app, "?");
    assert_eq!(press_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE), Effect::None);
    assert!(!render(&mut app, 100, 40).contains(" Keys "));
}

#[test]
fn the_footer_shows_send_and_help_first() {
    let mut app = both();
    let screen = render(&mut app, 100, 30);
    let footer = screen.lines().last().unwrap_or_default();
    let annotate = footer.find("note").unwrap_or(usize::MAX);
    let send = footer.find("S send").unwrap_or(0);
    let help = footer.find("? help").unwrap_or(0);
    assert!(annotate < send && send < help, "{footer}");
}

fn header_of(app: &mut App, width: u16) -> String {
    render(app, width, 24).lines().next().unwrap_or_default().to_owned()
}

fn footer_of(app: &mut App, width: u16) -> String {
    render(app, width, 24).lines().last().unwrap_or_default().to_owned()
}

#[test]
fn header_send_button_names_the_agent() {
    let pi = AgentState { agent: Some(Agent::Pi), safety: Safety::Clear { state: Some("idle".to_owned()) } };
    let mut app = App::new(pi, Screen::Viewer, vec![reply("r1", "one\n\ntwo\n")], &Config::default());
    let header = header_of(&mut app, 100);
    assert!(header.ends_with("[ Send ▸ ]"), "a dim button with no notes: {header}");
    press(&mut app, "cfirst\n");
    press(&mut app, "jcsecond\n");
    let header = header_of(&mut app, 100);
    assert!(header.ends_with("[ Send 2 → pi ▸ ]"), "{header}");
}

#[test]
fn footer_changes_with_the_mode() {
    let mut app = both();
    let block = footer_of(&mut app, 200);
    assert!(
        block.contains("block 1/1") && block.contains("j/k block") && block.ends_with("? help"),
        "{block}"
    );
    press(&mut app, "v");
    let text = footer_of(&mut app, 200);
    assert!(text.contains("o c d s note") && text.contains("esc clear"), "{text}");
    assert!(!text.contains("j/k block"), "{text}");
    press(&mut app, "c");
    let editor = footer_of(&mut app, 200);
    assert!(editor.contains("enter save") && editor.contains("esc cancel"), "{editor}");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    let rail = footer_of(&mut app, 200);
    assert!(rail.contains("enter go") && rail.contains("e edit"), "{rail}");
    press_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    let tree = footer_of(&mut app, 200);
    assert!(tree.contains("enter open") && tree.contains("/ filter") && tree.ends_with("? help"), "{tree}");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, "p");
    let list = footer_of(&mut app, 200);
    assert!(list.contains("enter open") && list.contains("esc close"), "{list}");
    assert!(!list.contains("/ filter"), "{list}");
}

#[test]
fn footer_drops_hints_from_the_end_but_keeps_help() {
    let mut app = both();
    let wide = footer_of(&mut app, 200);
    assert!(wide.contains("ctrl+p find"), "{wide}");
    let narrow = footer_of(&mut app, 60);
    assert!(narrow.ends_with("? help") && narrow.contains("j/k block"), "{narrow}");
    assert!(!narrow.contains("ctrl+p find"), "the last hints go first: {narrow}");
    assert!(narrow.chars().count() <= 60, "{narrow}");
}

#[test]
fn an_empty_document_shows_block_0_of_0() {
    let mut app = files_only();
    let footer = footer_of(&mut app, 80);
    assert!(footer.contains("block 0/0"), "{footer}");
    press(&mut app, "jkcvo");
    assert!(footer_of(&mut app, 80).contains("block 0/0"));
}

#[test]
fn a_new_text_closes_the_numbered_pick() {
    let mut app = both();
    press(&mut app, "cone\n");
    press(&mut app, "ctwo\n");
    press(&mut app, "e");
    assert!(render(&mut app, 80, 24).contains(" pick "), "two notes on the block open the pick");
    app.set_replies(vec![reply("r1", "new start\n\nalpha beta gamma\n"), reply("r2", "older reply text\n")]);
    assert!(!render(&mut app, 80, 24).contains(" pick "), "the ids of the pick can be stale");
    press(&mut app, "1");
    assert!(app.editor().is_none(), "1 does not edit a stale note");
}
