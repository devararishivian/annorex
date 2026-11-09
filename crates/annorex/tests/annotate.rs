//! Annotating: selections, the editor, the annotation list, and drafts.

#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use annorex::agent::Agent;
use annorex::config::Config;
use annorex::replies::Reply;
use annorex::review::Kind;
use annorex::safety::Safety;
use annorex::ui::{AgentState, App, Effect, Screen, draw};
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

fn app_with(text: &str, config: &Config) -> App {
    let mut app = App::new(state(), Screen::Viewer, vec![reply("r1", text)], config);
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    app
}

fn app(text: &str) -> App {
    app_with(text, &Config::default())
}

fn three_replies() -> App {
    let replies = vec![
        reply("r1", "alpha beta gamma\n"),
        reply("r2", "second reply text\n"),
        reply("r3", "third reply text\n"),
    ];
    let mut app = App::new(state(), Screen::Viewer, replies, &Config::default());
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

const LIST: &str = "- one\n- two\n- three\n";

#[test]
fn v_selection_makes_a_comment() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllllc");
    assert!(app.editor().is_some());
    press(&mut app, "Which page?\n");
    assert!(app.editor().is_none());
    let review = app.review().expect("review");
    assert_eq!(review.annotations.len(), 1);
    let a = &review.annotations[0];
    assert_eq!((a.kind, a.quote.as_str(), a.note.as_str()), (Kind::Comment, "alpha", "Which page?"));
    assert_eq!(a.range, 0..5);
}

#[test]
fn a_selection_ends_with_the_whole_last_character() {
    let mut app = app("a\u{e9}\u{301}z\n");
    press(&mut app, "vlc");
    press(&mut app, "x\n");
    assert_eq!(app.review().expect("review").annotations[0].quote, "a\u{e9}\u{301}");
}

#[test]
fn line_selection_makes_a_delete() {
    let mut app = app(LIST);
    press(&mut app, "Vjd\n");
    let a = &app.review().expect("review").annotations[0];
    assert_eq!(a.kind, Kind::Delete);
    assert_eq!(a.quote, "- one\n- two");
}

#[test]
fn no_selection_uses_the_cursor_line() {
    let mut app = app(LIST);
    press(&mut app, "jd\n");
    assert_eq!(app.review().expect("review").annotations[0].quote, "- two");
}

#[test]
fn a_selection_in_the_wrong_order_still_works() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "$vhhhc");
    press(&mut app, "x\n");
    assert_eq!(app.review().expect("review").annotations[0].quote, "amma");
}

#[test]
fn esc_clears_the_selection() {
    let mut app = app("alpha beta\n");
    press(&mut app, "vll");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, "dx\n");
    assert_eq!(app.review().expect("review").annotations[0].quote, "alpha beta");
}

#[test]
fn empty_comment_is_refused() {
    let mut app = app("alpha beta\n");
    press(&mut app, "c\n");
    assert!(app.editor().is_some(), "the editor stays open");
    assert!(app.status().is_some_and(|text| text.contains("comment")), "{:?}", app.status());
    assert!(app.review().is_none_or(annorex::review::Review::is_empty));
}

#[test]
fn replace_needs_the_new_text() {
    let mut app = app("alpha beta\n");
    press(&mut app, "vlls\n");
    assert!(app.editor().is_some());
    press(&mut app, "new\n");
    let a = &app.review().expect("review").annotations[0];
    assert_eq!((a.kind, a.quote.as_str(), a.replacement.as_deref()), (Kind::Replace, "alp", Some("new")));
}

#[test]
fn esc_cancels_the_editor() {
    let mut app = app("alpha beta\n");
    press(&mut app, "cabc");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.editor().is_none());
    assert!(app.review().is_none_or(annorex::review::Review::is_empty));
}

#[test]
fn tab_cycles_labels() {
    let config = Config {
        labels: vec!["question".into(), "looks good".into(), "must fix".into(), "needs test".into()],
        ..Config::default()
    };
    let mut app = app_with("alpha beta\n", &config);
    press(&mut app, "c");
    let mut seen = Vec::new();
    for _ in 0..5 {
        press_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        seen.push(app.editor().expect("editor").label.clone());
    }
    let expected = [Some("question"), Some("looks good"), Some("must fix"), Some("needs test"), None];
    assert_eq!(seen, expected.map(|label| label.map(str::to_owned)));
}

#[test]
fn a_label_alone_makes_a_valid_comment() {
    let mut app = app("alpha beta\n");
    press(&mut app, "c");
    press_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut app, "\n");
    let a = &app.review().expect("review").annotations[0];
    assert_eq!(a.label.as_deref(), Some("question"));
}

#[test]
fn ctrl_j_and_alt_enter_add_a_new_line() {
    let mut app = app("alpha beta\n");
    press(&mut app, "ca");
    press_key(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL);
    press(&mut app, "b");
    press_key(&mut app, KeyCode::Enter, KeyModifiers::ALT);
    press(&mut app, "c");
    assert_eq!(app.editor().expect("editor").text, "a\nb\nc");
    press(&mut app, "\n");
    assert_eq!(app.review().expect("review").annotations[0].note, "a\nb\nc");
}

#[test]
fn backspace_removes_the_last_character() {
    let mut app = app("alpha beta\n");
    press(&mut app, "cabc");
    press_key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(app.editor().expect("editor").text, "ab");
}

#[test]
fn each_change_asks_for_a_draft_save() {
    let mut app = app("alpha beta\n");
    assert_eq!(press_key(&mut app, KeyCode::Char('c'), KeyModifiers::NONE), Effect::SaveDraft);
    assert_eq!(press_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE), Effect::SaveDraft);
    assert_eq!(press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE), Effect::SaveDraft);
    assert_eq!(press_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE), Effect::None);
}

#[test]
fn annotation_list_goes_edits_and_deletes() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllc");
    press(&mut app, "first\n");
    press(&mut app, "$vhc");
    press(&mut app, "second\n");
    press(&mut app, "a");
    let screen = render(&mut app, 120, 24);
    assert!(screen.contains("1  l.1  comment") && screen.contains("2  l.1  comment"), "{screen}");
    press(&mut app, "j");
    press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.cursor(), 14, "enter moves the cursor to the start of the range");
    press(&mut app, "ae");
    assert_eq!(app.editor().expect("editor").text, "first");
    press(&mut app, "!\n");
    assert_eq!(app.review().expect("review").in_order()[0].note, "first!");
    press(&mut app, "ajx");
    let screen = render(&mut app, 120, 24);
    assert!(screen.contains("Delete annotation 2? y/n"), "{screen}");
    press(&mut app, "n");
    assert_eq!(app.review().expect("review").annotations.len(), 2);
    press(&mut app, "x");
    press(&mut app, "y");
    let review = app.review().expect("review");
    assert_eq!(review.annotations.len(), 1);
    assert_eq!(review.annotations[0].note, "first!");
}

#[test]
fn l_edits_the_annotation_at_the_cursor_or_makes_a_comment() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "L");
    assert_eq!(app.editor().expect("editor").kind, Kind::Comment);
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, "vllc");
    press(&mut app, "note\n");
    press(&mut app, "L");
    assert_eq!(app.editor().expect("editor").text, "note");
}

#[test]
fn each_reply_keeps_its_own_review() {
    let mut app = three_replies();
    press(&mut app, "cmine\n");
    press(&mut app, "pj");
    press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.review().is_none_or(annorex::review::Review::is_empty), "reply 2 has no annotation");
    press_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut app, "k");
    press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.review().expect("review").annotations.len(), 1);
}

#[test]
fn editor_text_survives_restart() {
    let mut first = app("alpha beta gamma\n");
    press(&mut first, "vllc");
    press(&mut first, "half a comm");
    let draft = first.draft(Agent::ClaudeCode, "s1").expect("draft");
    assert_eq!(draft.key.document, "r1");
    assert_eq!(draft.editing.as_ref().map(|e| e.text.as_str()), Some("half a comm"));

    let mut second = app("alpha beta gamma\n");
    second.restore("r1", draft);
    let editor = second.editor().expect("editor");
    assert_eq!(
        (editor.kind, editor.text.as_str(), editor.range.clone()),
        (Kind::Comment, "half a comm", 0..3)
    );
    press(&mut second, "\n");
    assert_eq!(second.review().expect("review").annotations[0].note, "half a comm");
}

#[test]
fn restore_checks_the_quotes_against_the_reply() {
    let mut first = app("alpha beta gamma\n");
    press(&mut first, "wvlllc");
    press(&mut first, "x\n");
    let draft = first.draft(Agent::ClaudeCode, "s1").expect("draft");
    let mut second = app("new line\nalpha beta gamma\n");
    second.restore("r1", draft);
    let a = &second.review().expect("review").annotations[0];
    assert_eq!((a.quote.as_str(), a.range.clone(), a.detached), ("beta", 15..19, false));
}

#[test]
fn a_new_change_makes_a_sent_draft_unsent() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "cx\n");
    app.mark_sent("2026-11-09T10:00:00Z".to_owned());
    assert_eq!(
        app.draft(Agent::ClaudeCode, "s").expect("draft").sent_at.as_deref(),
        Some("2026-11-09T10:00:00Z")
    );
    assert!(!render(&mut app, 80, 12).contains("unsent"));
    press(&mut app, "jcy\n");
    assert!(app.draft(Agent::ClaudeCode, "s").expect("draft").sent_at.is_none());
    assert!(render(&mut app, 80, 12).contains("unsent 1"));
}

#[test]
fn header_lists_unsent_replies() {
    let mut app = three_replies();
    press(&mut app, "cone\n");
    press(&mut app, "pjj");
    press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    press(&mut app, "cthree\n");
    let screen = render(&mut app, 120, 24);
    let header = screen.lines().next().expect("header");
    assert!(header.contains("unsent 1,3"), "{header}");
    assert!(screen.lines().any(|line| line.contains(" 1●")), "{screen}");
}

#[test]
fn annotated_ranges_are_underlined_and_the_selection_is_reversed() {
    use ratatui::style::Modifier;
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllc");
    press(&mut app, "x\n");
    press(&mut app, "wvl");
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).expect("terminal");
    terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let has = |x: u16, modifier| buffer.cell((x, 1)).expect("cell").style().add_modifier.contains(modifier);
    assert!(has(2, Modifier::UNDERLINED) && has(4, Modifier::UNDERLINED), "alp is annotated");
    assert!(!has(5, Modifier::UNDERLINED), "the space is not");
    assert!(has(8, Modifier::REVERSED) && has(9, Modifier::REVERSED), "the selection is reversed");
    assert!(!has(10, Modifier::REVERSED));
}

fn two_notes() -> App {
    let mut app = app("alpha beta gamma\n\n- one\n- two\n");
    press(&mut app, "vllc");
    press(&mut app, "Which page?\n");
    press(&mut app, "jjVjd");
    press_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    press_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    press_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut app, "Not needed.\n");
    app
}

#[test]
fn snapshot_annotations_side_120x40() {
    let mut app = two_notes();
    press(&mut app, "a");
    insta::assert_snapshot!(render(&mut app, 120, 40));
}

#[test]
fn snapshot_editor_80x24() {
    let mut app = app("We change the login page.\n\n- one\n- two\n");
    press(&mut app, "wwvllllc");
    press(&mut app, "Which page?");
    press_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    insta::assert_snapshot!(render(&mut app, 80, 24));
}
