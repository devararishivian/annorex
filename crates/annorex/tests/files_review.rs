//! Reviewing many files: the open files, the combined export, and the quit question.

#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use std::path::PathBuf;

use annorex::agent::Agent;
use annorex::config::Config;
use annorex::files::{LoadedFile, Stamp};
use annorex::safety::Safety;
use annorex::ui::{AgentState, App, Effect, draw};
use annorex::watch::{Change, ChangeKind};
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

/// A work directory with `README.md`, `src/main.rs`, and `src/lib.rs`, and an app that scanned it.
fn tree_app(config: &Config) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::create_dir(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("README.md"), "# Readme\n").expect("readme");
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").expect("main");
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn lib() {}\n").expect("lib");
    let mut app = App::new_files(state(), dir.path().to_path_buf(), config);
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    app.set_scan(annorex::files::scan(dir.path()));
    (dir, app)
}

fn ctrl(app: &mut App, c: char) {
    press_key(app, KeyCode::Char(c), KeyModifiers::CONTROL);
}

#[test]
fn tree_lists_expands_and_opens() {
    let (_dir, mut app) = tree_app(&Config::default());
    press(&mut app, "T");
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("Tree") && screen.contains("src") && screen.contains("README.md"), "{screen}");
    assert!(!screen.contains("main.rs"), "a directory starts closed: {screen}");
    press(&mut app, "\n"); // src is first: directories come before files
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("main.rs") && screen.contains("lib.rs"), "{screen}");
    press(&mut app, "h");
    assert!(!render(&mut app, 120, 20).contains("main.rs"), "h closes the directory");
    press(&mut app, "l");
    press(&mut app, "jj\n"); // lib.rs, main.rs
    assert_eq!(app.shown_file().and_then(|path| path.to_str()), Some("src/main.rs"));
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("fn main() {}"), "{screen}");
}

#[test]
fn tree_shows_truncated_notice() {
    let (dir, mut app) = tree_app(&Config::default());
    app.set_scan(annorex::files::scan_with(dir.path(), 2));
    press(&mut app, "T");
    let screen = render(&mut app, 120, 20);
    let footer = screen.lines().last().expect("footer");
    assert!(footer.contains("The tree shows the first 20,000 entries."), "{footer}");
}

#[test]
fn tree_shows_scanning_before_the_scan_ends() {
    let mut app = files_app();
    press(&mut app, "T");
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("Scanning…"), "{screen}");
}

#[test]
fn tree_filter_ranks_full_paths() {
    let (_dir, mut app) = tree_app(&Config::default());
    press(&mut app, "T/lib");
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("src/lib.rs") && !screen.contains("README.md"), "{screen}");
    press(&mut app, "\n");
    assert_eq!(app.shown_file().and_then(|path| path.to_str()), Some("src/lib.rs"));
}

#[test]
fn finder_filters_while_typing_and_opens_the_best_match() {
    let (_dir, mut app) = tree_app(&Config::default());
    ctrl(&mut app, 'p');
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("README.md") && screen.contains("src/main.rs"), "{screen}");
    press(&mut app, "main");
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("src/main.rs") && !screen.contains("README.md"), "{screen}");
    assert!(screen.contains("> main"), "the query shows: {screen}");
    press(&mut app, "\n");
    assert_eq!(app.shown_file().and_then(|path| path.to_str()), Some("src/main.rs"));
    // Arrows and ctrl+n / ctrl+p move; esc closes.
    ctrl(&mut app, 'p');
    press(&mut app, "s");
    ctrl(&mut app, 'n');
    press_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    press_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    press_key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!render(&mut app, 120, 20).contains("Find file"), "esc closes the finder");
}

#[test]
fn finder_with_no_match_shows_none() {
    let (_dir, mut app) = tree_app(&Config::default());
    ctrl(&mut app, 'p');
    press(&mut app, "zzzq");
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("no match"), "{screen}");
    press(&mut app, "\n");
    assert_eq!(app.shown_file(), None);
}

#[test]
fn marks_show_files_with_unsent_drafts() {
    let (_dir, mut app) = tree_app(&Config::default());
    app.set_draft_marks(&["file:README.md".to_owned()]);
    press(&mut app, "T");
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("● README.md"), "{screen}");
    assert!(!screen.contains("● src"), "{screen}");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    ctrl(&mut app, 'p');
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("● README.md") && !screen.contains("● src/main.rs"), "{screen}");
}

#[test]
fn open_files_with_unsent_notes_are_marked_too() {
    let (_dir, mut app) = tree_app(&Config::default());
    ctrl(&mut app, 'p');
    press(&mut app, "main\n");
    press(&mut app, "cnote\n");
    press(&mut app, "T");
    press(&mut app, "\n"); // expand src
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("● main.rs") && !screen.contains("● lib.rs"), "{screen}");
}

#[test]
fn rebound_keys_open_the_tree_and_the_finder() {
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("config.toml");
    std::fs::write(&file, "[keys]\ntree = \"t\"\nfind_file = \"ctrl+o\"\n").expect("config");
    let config = Config::load(&file).expect("config");
    let (_work, mut app) = tree_app(&config);
    press(&mut app, "t");
    assert!(render(&mut app, 120, 20).contains("Tree"));
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    ctrl(&mut app, 'o');
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("Find file") && screen.contains("README.md"), "{screen}");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, "T");
    assert!(!render(&mut app, 120, 20).contains("Tree"), "the old key does nothing");
    let footer = render(&mut app, 120, 20).lines().last().expect("footer").to_owned();
    assert!(footer.contains("t tree") && footer.contains("ctrl+o find file"), "{footer}");
}

#[test]
fn snapshot_tree_side_120x40() {
    let (_dir, mut app) = tree_app(&Config::default());
    app.set_draft_marks(&["file:README.md".to_owned()]);
    press(&mut app, "T\n");
    insta::assert_snapshot!(render(&mut app, 120, 40));
}

#[test]
fn snapshot_finder_80x24() {
    let (_dir, mut app) = tree_app(&Config::default());
    ctrl(&mut app, 'p');
    press(&mut app, "rs");
    insta::assert_snapshot!(render(&mut app, 80, 24));
}

#[test]
fn snapshot_tree_overlay_50x20() {
    let (_dir, mut app) = tree_app(&Config::default());
    press(&mut app, "T\n");
    insta::assert_snapshot!(render(&mut app, 50, 20));
}

/// An app on a real work directory with `name` written and opened.
fn real_app(name: &str, text: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::write(dir.path().join(name), text).expect("write");
    let mut app = App::new_files(state(), dir.path().to_path_buf(), &Config::default());
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    app.open_file(annorex::files::load(dir.path(), std::path::Path::new(name)).expect("load"));
    (dir, app)
}

fn change(path: &str, kind: ChangeKind) -> Change {
    Change { path: PathBuf::from(path), kind }
}

#[test]
fn changed_file_shows_the_banner_and_r_clears_it() {
    let (dir, mut app) = real_app("a.txt", "alpha beta\n");
    assert!(!render(&mut app, 80, 12).contains("changed on disk"));
    std::fs::write(dir.path().join("a.txt"), "new text\nalpha beta\n").expect("change");
    app.file_changed(&change("a.txt", ChangeKind::Modified));
    let screen = render(&mut app, 80, 12);
    assert!(screen.contains("a.txt changed on disk. Press r to read it again."), "{screen}");
    assert!(screen.contains("alpha beta") && !screen.contains("new text"), "the old text stays: {screen}");
    press(&mut app, "p");
    assert!(render(&mut app, 80, 12).contains("≠ a.txt"), "the list marks the file");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, "r");
    let screen = render(&mut app, 80, 12);
    assert!(!screen.contains("changed on disk") && screen.contains("new text"), "{screen}");
}

#[test]
fn changed_file_reanchors_and_detaches() {
    let (dir, mut app) = real_app("a.txt", "alpha beta gamma\n");
    press(&mut app, "0vllllcfirst\n");
    press(&mut app, "wwvllllcsecond\n"); // "gamma"
    std::fs::write(dir.path().join("a.txt"), "intro line\nalpha beta\n").expect("change");
    app.file_changed(&change("a.txt", ChangeKind::Modified));
    press(&mut app, "r");
    let notes = &app.review().expect("review").annotations;
    assert_eq!(notes.len(), 2, "nothing is deleted");
    assert_eq!(notes[0].range, 11..16, "the note follows its text");
    assert!(!notes[0].detached);
    assert!(notes[1].detached, "the text of the second note is gone");
    let export = app.export();
    assert!(export.contains("text not found"), "{export}");
    assert!(export.contains("first") && export.contains("second"), "{export}");
}

#[test]
fn deleted_file_keeps_notes_and_exports() {
    let (dir, mut app) = real_app("a.txt", "alpha beta\n");
    press(&mut app, "0vllllcnote\n");
    std::fs::remove_file(dir.path().join("a.txt")).expect("remove");
    app.file_changed(&change("a.txt", ChangeKind::Deleted));
    let screen = render(&mut app, 80, 12);
    assert!(screen.contains("a.txt was deleted. Your notes are kept."), "{screen}");
    press(&mut app, "r");
    assert_eq!(app.status(), Some("The file is gone. Your notes are kept."));
    assert!(app.export().contains("note"), "{}", app.export());
    assert!(render(&mut app, 80, 12).contains("alpha beta"), "the text stays");
}

#[test]
fn editor_open_during_a_change_survives_the_reload() {
    let (dir, mut app) = real_app("a.txt", "alpha beta\n");
    press(&mut app, "0vllllcwip");
    std::fs::write(dir.path().join("a.txt"), "x\nalpha beta\n").expect("change");
    app.file_changed(&change("a.txt", ChangeKind::Modified));
    let screen = render(&mut app, 80, 12);
    assert!(screen.contains("wip") && screen.contains("changed on disk"), "{screen}");
    let loaded = annorex::files::load(dir.path(), std::path::Path::new("a.txt")).expect("load");
    app.reload_file(&annorex::ui::file_id(std::path::Path::new("a.txt")), loaded);
    press(&mut app, "\n");
    let note = &app.review().expect("review").annotations[0];
    assert_eq!(note.range, 2..7, "the editor range moved with its text");
    assert_eq!(note.note, "wip");
}

#[test]
fn banner_snapshot_80x24() {
    let (dir, mut app) = real_app("a.txt", "alpha beta\nsecond line\n");
    press(&mut app, "0vllllcnote\n");
    std::fs::write(dir.path().join("a.txt"), "changed\n").expect("change");
    app.file_changed(&change("a.txt", ChangeKind::Modified));
    insta::assert_snapshot!(render(&mut app, 80, 24));
}

#[test]
fn restored_m2_draft_with_sent_at_has_no_unsent_notes() {
    use annorex::draft::{Draft, DraftKey};
    use annorex::review::Kind;
    let mut app = two_files();
    let mut draft = Draft::new(
        DraftKey { agent: Agent::ClaudeCode, session: "s".to_owned(), document: "file:a.md".to_owned() },
        "a.md",
    );
    draft.review.add("# Title\n\nalpha text\n", Kind::Comment, 0..7);
    draft.sent_at = Some("2026-11-09T10:00:00Z".to_owned());
    app.restore("file:a.md", draft);
    assert_eq!(app.unsent_replies(), Vec::<usize>::new());
    assert_eq!(app.export(), "", "nothing goes to the agent twice");
}

#[test]
fn closing_a_file_is_refused_when_the_draft_cannot_be_saved() {
    let mut app = two_files();
    press(&mut app, "cnote\n");
    app.set_save_error(Some("disk full".to_owned()));
    press(&mut app, "W");
    press(&mut app, "y");
    assert_eq!(app.shown_file().and_then(|p| p.to_str()), Some("a.md"), "the file stays open");
    assert!(app.review().is_some_and(|r| !r.annotations.is_empty()), "the notes stay");
}

#[test]
fn stale_watch_message_does_not_bring_the_banner_back_after_r() {
    let (dir, mut app) = real_app("a.txt", "alpha beta\n");
    std::fs::write(dir.path().join("a.txt"), "new text\nalpha beta\n").expect("change");
    app.file_changed(&change("a.txt", ChangeKind::Modified));
    press(&mut app, "r");
    // The thread sent this message before the user read the file again.
    app.file_changed(&change("a.txt", ChangeKind::Modified));
    let screen = render(&mut app, 80, 12);
    assert!(!screen.contains("changed on disk"), "{screen}");
}

#[test]
fn finder_opened_before_the_scan_fills_when_the_scan_ends() {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::write(dir.path().join("README.md"), "# Readme\n").expect("readme");
    let mut app = App::new_files(state(), dir.path().to_path_buf(), &Config::default());
    ctrl(&mut app, 'p');
    assert!(render(&mut app, 80, 12).contains("Scanning…"), "the finder says that the scan is running");
    app.set_scan(annorex::files::scan(dir.path()));
    let screen = render(&mut app, 80, 12);
    assert!(screen.contains("README.md") && !screen.contains("Scanning…"), "{screen}");
}

#[test]
fn header_and_nothing_to_send_look_at_all_open_files() {
    let mut app = two_files(); // a.md on the screen
    press(&mut app, "cnote\n");
    app.mark_sent("2026-11-09T10:00:00Z".to_owned());
    assert!(render(&mut app, 100, 12).contains("[Sent 10:00]"));
    // Show b.txt, which has no notes: the notes of a.md were sent, so the header still says so.
    press(&mut app, "[");
    assert!(render(&mut app, 100, 12).contains("[Sent 10:00]"), "sent state is for all files");
    press(&mut app, "S");
    assert_eq!(app.status(), Some("Nothing new to send. Press Y to copy all notes."));
    // A new note in b.txt makes the header a send button again.
    press(&mut app, "cmore\n");
    let screen = render(&mut app, 100, 12);
    assert!(screen.contains("[Send 2") && !screen.contains("[Sent"), "{screen}");
}

#[test]
fn a_directory_on_the_command_line_opens_the_tree_there() {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::create_dir(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("README.md"), "# Readme\n").expect("readme");
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").expect("main");
    let mut app = App::new_files(state(), dir.path().to_path_buf(), &Config::default());
    app.reveal_dir(std::path::Path::new("src"));
    app.set_scan(annorex::files::scan(dir.path()));
    let screen = render(&mut app, 120, 20);
    assert!(screen.contains("Tree") && screen.contains("main.rs"), "the directory is open: {screen}");
    press(&mut app, "\n"); // the cursor is on the directory: Enter closes it
    assert!(!render(&mut app, 120, 20).contains("main.rs"));
}
