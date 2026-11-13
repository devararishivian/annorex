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
    // The view starts without a focus: `j` focuses the first block.
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
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
    // The view starts without a focus: `j` focuses the first block.
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
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
fn o_adds_looks_good_without_an_editor() {
    let mut app = app("alpha beta gamma\n");
    let effect = press_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
    assert!(matches!(effect, Effect::None | Effect::SaveDraft));
    press(&mut app, "llll");
    let effect = press_key(&mut app, KeyCode::Char('o'), KeyModifiers::NONE);
    assert!(matches!(effect, Effect::SaveDraft));
    assert!(app.editor().is_none());
    let review = app.review().expect("review");
    assert_eq!(review.annotations.len(), 1);
    assert_eq!(
        (review.annotations[0].kind, review.annotations[0].quote.as_str()),
        (Kind::LooksGood, "alpha")
    );
}

#[test]
fn a_second_looks_good_on_the_same_range_is_refused() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllllo0");
    press(&mut app, "vllllo");
    assert_eq!(app.review().expect("review").annotations.len(), 1);
    assert_eq!(app.status(), Some("Already saved."));
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
    press(&mut app, "Vjd");
    let a = &app.review().expect("review").annotations[0];
    assert_eq!(a.kind, Kind::Delete);
    assert_eq!(a.quote, "- one\n- two");
}

#[test]
fn no_selection_uses_the_cursor_line() {
    let mut app = app(LIST);
    // Text mode: without a selection a note covers the line of the cursor.
    press(&mut app, "0jd");
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
    press(&mut app, "d");
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
fn box_has_no_label_line() {
    let mut app = app("alpha beta\n");
    press(&mut app, "cx");
    let screen = render(&mut app, 80, 24);
    assert!(!screen.contains("label:"), "{screen}");
    assert!(!screen.contains("Comment on"), "{screen}");
    assert!(!screen.contains("tab label"), "{screen}");
}

#[test]
fn tab_does_not_set_a_label() {
    let mut app = app("alpha beta\n");
    press(&mut app, "cx");
    press_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    let editor = app.editor().expect("editor");
    assert_eq!((editor.label.clone(), editor.text.as_str()), (None, "x"));
    press(&mut app, "\n");
    assert_eq!(app.review().expect("review").annotations[0].label, None);
}

#[test]
fn box_title_names_kind_and_line() {
    let mut app = app("alpha beta\n\nsecond block\n");
    press(&mut app, "jc");
    let screen = render(&mut app, 80, 24);
    assert!(screen.contains("Comment · line 3"), "{screen}");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, "vlls");
    assert!(render(&mut app, 80, 24).contains("Replace · line 3"));
}

#[test]
fn empty_box_shows_hint() {
    let mut app = app("alpha beta\n");
    press(&mut app, "c");
    assert!(render(&mut app, 80, 24).contains("Write your comment"));
    press(&mut app, "x");
    assert!(!render(&mut app, 80, 24).contains("Write your comment"));
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, "vlls");
    assert!(render(&mut app, 80, 24).contains("Write the new text"));
}

fn thirty_rows() -> String {
    (1..=30).map(|n| format!("row {n}")).collect::<Vec<_>>().join("\n")
}

/// The row of the top border and the row of the bottom border of the note box.
fn box_rows(screen: &str) -> (usize, usize) {
    let top = screen
        .lines()
        .position(|line| line.contains('┌') && line.contains("Comment"))
        .unwrap_or_else(|| panic!("no box in\n{screen}"));
    (top, row_of(screen, " save "))
}

#[test]
fn long_paste_shows_its_end() {
    let mut app = app("alpha beta\n");
    press(&mut app, "c");
    app.handle_paste(&thirty_rows());
    let screen = render(&mut app, 80, 24);
    let (top, bottom) = box_rows(&screen);
    assert_eq!(bottom - top - 1, 8, "8 text rows:\n{screen}");
    assert!(screen.lines().nth(bottom - 1).is_some_and(|line| line.contains("row 30")), "{screen}");
    assert!(screen.lines().nth(top + 1).is_some_and(|line| line.contains("row 23")), "{screen}");
    assert!(!screen.contains("row 22"), "{screen}");
}

#[test]
fn pgup_scrolls_box() {
    let mut app = app("alpha beta\n");
    press(&mut app, "c");
    app.handle_paste(&thirty_rows());
    render(&mut app, 80, 24);
    press_key(&mut app, KeyCode::PageUp, KeyModifiers::NONE);
    let screen = render(&mut app, 80, 24);
    assert!(screen.contains("row 22") && screen.contains("row 15"), "{screen}");
    assert!(!screen.contains("row 30"), "{screen}");
    press_key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    assert!(render(&mut app, 80, 24).contains("row 30"));
    press_key(&mut app, KeyCode::PageUp, KeyModifiers::NONE);
    press(&mut app, "z");
    assert!(render(&mut app, 80, 24).contains("row 30"), "typing follows the end again");
}

#[test]
fn wheel_over_box_scrolls() {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let mut app = app("alpha beta\n");
    press(&mut app, "c");
    app.handle_paste(&thirty_rows());
    let screen = render(&mut app, 80, 24);
    let row = row_of(&screen, "row 30");
    let wheel = |kind| MouseEvent {
        kind,
        column: 10,
        row: u16::try_from(row).expect("row"),
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(wheel(MouseEventKind::ScrollUp));
    let screen = render(&mut app, 80, 24);
    assert!(screen.contains("row 20") && !screen.contains("row 30"), "{screen}");
    app.handle_mouse(wheel(MouseEventKind::ScrollDown));
    assert!(render(&mut app, 80, 24).contains("row 30"));
    assert!(app.editor().is_some(), "the wheel keeps the box open");
}

#[test]
fn hint_says_shift_enter_when_supported() {
    let mut app = app("alpha beta\n");
    app.shift_enter = true;
    press(&mut app, "c");
    let screen = render(&mut app, 80, 24);
    assert!(screen.contains("shift+enter new line") && !screen.contains("ctrl+j"), "{screen}");
}

#[test]
fn hint_says_ctrl_j_when_not() {
    let mut app = app("alpha beta\n");
    press(&mut app, "c");
    let screen = render(&mut app, 80, 24);
    assert!(screen.contains("ctrl+j new line") && !screen.contains("shift+enter"), "{screen}");
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
    // At 110 columns the window has no card rail, so the list opens beside the document.
    let _ = render(&mut app, 110, 24);
    press(&mut app, "a");
    let screen = render(&mut app, 110, 24);
    assert!(screen.contains("1  l.1  comment") && screen.contains("2  l.1  comment"), "{screen}");
    press(&mut app, "j");
    press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.cursor(), 14, "enter moves the cursor to the start of the range");
    // The list starts at the note at the cursor: the second one. `k` goes to the first.
    press(&mut app, "ake");
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
    press(&mut app, "pk");
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
    app.mark_sent("2026-11-09T10:00:00Z");
    assert_eq!(
        app.draft(Agent::ClaudeCode, "s").expect("draft").sent_at.as_deref(),
        Some("2026-11-09T10:00:00Z")
    );
    assert!(!render(&mut app, 80, 12).contains("unsent"));
    press(&mut app, "jcy\n");
    assert!(app.draft(Agent::ClaudeCode, "s").expect("draft").sent_at.is_none());
    assert!(render(&mut app, 100, 12).contains("unsent in reply 1"));
}

#[test]
fn header_lists_unsent_replies() {
    let mut app = three_replies();
    press(&mut app, "cone\n");
    press(&mut app, "pjj");
    press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    // The reply that opens has no focus yet: `j` focuses its block.
    press(&mut app, "jcthree\np");
    let screen = render(&mut app, 120, 24);
    let header = screen.lines().next().expect("header");
    assert!(header.contains("unsent in reply 1,3"), "{header}");
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
    assert!(has(1, Modifier::UNDERLINED) && has(3, Modifier::UNDERLINED), "alp is annotated");
    assert!(!has(4, Modifier::UNDERLINED), "the space is not");
    assert!(has(7, Modifier::REVERSED) && has(8, Modifier::REVERSED), "the selection is reversed");
    assert!(!has(9, Modifier::REVERSED));
}

fn two_notes() -> App {
    let mut app = app("alpha beta gamma\n\n- one\n- two\n");
    press(&mut app, "vllc");
    press(&mut app, "Which page?\n");
    press(&mut app, "jjVjd");
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
    insta::assert_snapshot!(render(&mut app, 80, 24));
}

fn state_of(agent: Agent, safety: Safety) -> AgentState {
    AgentState { agent: Some(agent), safety }
}

fn annotated(agent: Agent, safety: Safety) -> App {
    let mut app = App::new(
        state_of(agent, safety),
        Screen::Viewer,
        vec![reply("r1", "alpha beta\n")],
        &Config::default(),
    );
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    press(&mut app, "jcnote\n");
    app
}

fn header(app: &mut App) -> String {
    render(app, 120, 12).lines().next().expect("header").to_owned()
}

#[test]
fn send_keys_ask_the_caller() {
    use annorex::deliver::DeliveryMode;
    let mut app = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    assert_eq!(
        press_key(&mut app, KeyCode::Char('S'), KeyModifiers::SHIFT),
        Effect::Send(DeliveryMode::Send)
    );
    assert_eq!(
        press_key(&mut app, KeyCode::Char('P'), KeyModifiers::SHIFT),
        Effect::Send(DeliveryMode::PasteOnly)
    );
    assert_eq!(press_key(&mut app, KeyCode::Char('Y'), KeyModifiers::SHIFT), Effect::Copy);
}

#[test]
fn nothing_to_send_says_so() {
    let mut app = app("alpha beta\n");
    assert_eq!(press_key(&mut app, KeyCode::Char('S'), KeyModifiers::SHIFT), Effect::None);
    assert!(app.status().is_some_and(|text| text.contains("Nothing to send")), "{:?}", app.status());
}

#[test]
fn send_with_note_puts_the_note_first() {
    let mut app = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    press(&mut app, "M");
    assert!(app.editor().is_some_and(|editor| editor.general));
    press(&mut app, "Please fix all.");
    assert_eq!(press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE), Effect::SendWithNote);
    let export = app.export();
    assert!(export.starts_with("Please fix all.\n\n# Annotations on last reply\n"), "{export}");
    assert!(export.contains("## Annotation 1 (line 1)"), "{export}");
}

#[test]
fn send_marks_the_draft_sent_and_a_change_makes_it_unsent_again() {
    use annorex::send::{SendResult, apply_result};
    let mut app = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    assert!(header(&mut app).contains("[ Send 1 → claude ▸ ]"), "{}", header(&mut app));
    assert!(apply_result(&mut app, SendResult::Sent, "2026-11-09T14:30:00Z"));
    let draft = app.draft(Agent::ClaudeCode, "s").expect("draft");
    assert_eq!(draft.sent_at.as_deref(), Some("2026-11-09T14:30:00Z"));
    assert_eq!(draft.review.annotations.len(), 1, "the annotations stay");
    assert!(app.status().is_some_and(|text| text.contains("If the agent prompt had text")));
    let top = header(&mut app);
    assert!(top.contains("[ Sent 14:30 ]") && !top.contains("unsent"), "{top}");
    press(&mut app, "cmore\n");
    assert!(app.draft(Agent::ClaudeCode, "s").expect("draft").sent_at.is_none());
    assert!(header(&mut app).contains("[ Send 1 → claude ▸ ]"));
}

#[test]
fn a_refused_send_keeps_the_draft_unsent() {
    use annorex::send::{SendResult, apply_result};
    let mut app = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    let before = app.draft(Agent::ClaudeCode, "s").expect("draft").review;
    assert!(!apply_result(&mut app, SendResult::Refused("Not sent: x".to_owned()), "2026-11-09T14:30:00Z"));
    let draft = app.draft(Agent::ClaudeCode, "s").expect("draft");
    assert_eq!((draft.sent_at, draft.review), (None, before));
    assert_eq!(app.status(), Some("Not sent: x"));
}

#[test]
fn sending_clears_the_general_note() {
    use annorex::send::{SendResult, apply_result};
    let mut app = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    press(&mut app, "Mhello\n");
    apply_result(&mut app, SendResult::Sent, "2026-11-09T14:30:00Z");
    assert!(!app.export().contains("hello"));
}

#[test]
fn working_state_shows_queued_or_steers() {
    let working = || Safety::Clear { state: Some("working".to_owned()) };
    let mut claude = annotated(Agent::ClaudeCode, working());
    assert!(header(&mut claude).contains("[ Send 1 · queued → claude ▸ ]"), "{}", header(&mut claude));
    for agent in [Agent::Pi, Agent::OpenCode] {
        let mut app = annotated(agent, working());
        assert!(header(&mut app).contains("[ Send 1 · steers → "), "{}", header(&mut app));
    }
}

#[test]
fn blocked_state_dims_the_button() {
    use ratatui::style::Modifier;
    let blocked = Safety::Blocked { kind: Some("permission".to_owned()), msg: None };
    let mut app = annotated(Agent::ClaudeCode, blocked);
    let mut terminal = Terminal::new(TestBackend::new(120, 12)).expect("terminal");
    terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let x = (0..120)
        .rev()
        .find(|&x| buffer.cell((x, 0)).is_some_and(|cell| cell.symbol() == "["))
        .expect("button");
    assert!(buffer.cell((x + 1, 0)).expect("cell").style().add_modifier.contains(Modifier::DIM));
    let mut idle = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    let mut terminal = Terminal::new(TestBackend::new(120, 12)).expect("terminal");
    terminal.draw(|frame| draw(frame, &mut idle)).expect("draw");
    let buffer = terminal.backend().buffer();
    assert!(!buffer.cell((x + 1, 0)).expect("cell").style().add_modifier.contains(Modifier::DIM));
}

#[test]
fn quit_with_unsent_notes_asks_once() {
    let mut app = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    assert_eq!(press_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE), Effect::None);
    let screen = render(&mut app, 120, 12);
    assert!(screen.contains("Unsent notes. s send · k keep the draft · d discard · esc back"), "{screen}");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!render(&mut app, 120, 12).contains("Unsent notes."));
    assert_eq!(press_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE), Effect::None);
    assert_eq!(press_key(&mut app, KeyCode::Char('k'), KeyModifiers::NONE), Effect::Quit);
}

#[test]
fn quit_prompt_send_and_discard() {
    use annorex::deliver::DeliveryMode;
    use annorex::send::{SendResult, apply_result};
    let mut app = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    press(&mut app, "q");
    assert_eq!(press_key(&mut app, KeyCode::Char('s'), KeyModifiers::NONE), Effect::Send(DeliveryMode::Send));
    assert!(!app.should_exit());
    apply_result(&mut app, SendResult::Refused("Not sent".to_owned()), "t");
    assert!(!app.should_exit(), "a refused send does not quit");
    press(&mut app, "q");
    press(&mut app, "s");
    apply_result(&mut app, SendResult::Sent, "2026-11-09T14:30:00Z");
    assert!(app.should_exit(), "a sent review quits");

    let mut other = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    press(&mut other, "q");
    assert_eq!(press_key(&mut other, KeyCode::Char('d'), KeyModifiers::NONE), Effect::Discard);
    assert!(!other.should_exit(), "the caller removes the draft file first");
    other.quit_if_pending();
    assert!(other.should_exit());
    assert!(other.review().is_none());
}

#[test]
fn quit_after_send_does_not_ask() {
    use annorex::send::{SendResult, apply_result};
    let mut app = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    apply_result(&mut app, SendResult::Sent, "2026-11-09T14:30:00Z");
    assert_eq!(press_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE), Effect::Quit);
}

#[test]
fn quit_without_notes_does_not_ask() {
    let mut app = app("alpha beta\n");
    assert_eq!(press_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE), Effect::Quit);
}

#[test]
fn discard_removes_the_review_after_y() {
    let mut app = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    assert_eq!(press_key(&mut app, KeyCode::Char('X'), KeyModifiers::SHIFT), Effect::None);
    assert!(render(&mut app, 120, 12).contains("Discard the notes of this reply? y/n"));
    assert_eq!(press_key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE), Effect::None);
    assert!(app.review().is_some_and(|review| !review.is_empty()));
    press_key(&mut app, KeyCode::Char('X'), KeyModifiers::SHIFT);
    assert_eq!(press_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE), Effect::Discard);
    assert!(app.review().is_none());
    assert!(!header(&mut app).contains("unsent"));
}

#[test]
fn copy_gives_the_export_to_the_clipboard() {
    use annorex::clipboard::{Clipboard, CopyError, copy_review};
    struct Fake(Vec<String>, Result<(), CopyError>);
    impl Clipboard for Fake {
        fn copy(&mut self, text: &str) -> Result<(), CopyError> {
            self.0.push(text.to_owned());
            self.1.clone()
        }
    }
    let mut app = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    let mut fake = Fake(Vec::new(), Ok(()));
    copy_review(&mut app, &mut fake);
    assert_eq!(fake.0, [app.export()]);
    assert_eq!(app.status(), Some("Copied 1 annotation."));
    let mut failing = Fake(Vec::new(), Err(CopyError::failed("no pbcopy")));
    copy_review(&mut app, &mut failing);
    assert_eq!(app.status(), Some("Copy failed: no pbcopy"));
    let mut partial = Fake(Vec::new(), Err(CopyError::terminal_only("no pbcopy")));
    copy_review(&mut app, &mut partial);
    let status = app.status().expect("status");
    assert!(status.contains("terminal only") && status.contains("no pbcopy"), "{status}");
    let mut empty = self::app("alpha\n");
    let mut none = Fake(Vec::new(), Ok(()));
    copy_review(&mut empty, &mut none);
    assert_eq!(none.0, Vec::<String>::new());
    assert!(empty.status().is_some_and(|text| text.contains("Nothing to copy")));
}

#[test]
fn the_general_note_editor_survives_a_restart() {
    let mut first = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    press(&mut first, "Mhalf a no");
    let draft = first.draft(Agent::ClaudeCode, "s").expect("draft");
    let mut second = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    second.restore("r1", draft);
    let editor = second.editor().expect("editor");
    assert!(editor.general);
    assert_eq!(editor.text, "half a no");
}

#[test]
fn editing_an_annotation_survives_a_restart_without_a_duplicate() {
    let mut first = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    press(&mut first, "ae more");
    let draft = first.draft(Agent::ClaudeCode, "s").expect("draft");
    let mut second = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    second.restore("r1", draft);
    assert!(second.editor().is_some());
    press_key(&mut second, KeyCode::Enter, KeyModifiers::NONE);
    let review = second.review().expect("review");
    assert_eq!(review.annotations.len(), 1, "{:?}", review.annotations);
    assert!(review.annotations[0].note.ends_with(" more"), "{:?}", review.annotations[0].note);
}

#[test]
fn a_paste_goes_into_the_editor_as_text() {
    let mut app = app("alpha beta\n");
    press(&mut app, "c");
    let effect = app.handle_paste("foo()\r\nSee docs\n\x1b[31mXy\u{7}");
    assert_eq!(effect, Effect::SaveDraft);
    assert_eq!(app.editor().expect("editor").text, "foo()\nSee docs\n[31mXy");
    assert!(app.review().is_none_or(annorex::review::Review::is_empty), "a paste never saves");
}

#[test]
fn a_paste_outside_the_editor_does_nothing() {
    let mut app = annotated(Agent::ClaudeCode, Safety::Clear { state: Some("idle".to_owned()) });
    assert_eq!(app.handle_paste("S\nX\ny"), Effect::None);
    assert_eq!(app.review().expect("review").annotations.len(), 1);
}

fn idle() -> Safety {
    Safety::Clear { state: Some("idle".to_owned()) }
}

#[test]
fn export_skips_sent_annotations_and_a_change_makes_one_unsent() {
    let mut app = annotated(Agent::ClaudeCode, idle());
    app.finish_send("2026-11-09T11:00:00Z");
    assert!(app.export().is_empty(), "everything was sent");
    press(&mut app, "cother\n");
    let export = app.export();
    assert!(export.contains("> other") && !export.contains("> note"), "{export}");
    let all = app.export_all();
    assert!(all.contains("> other") && all.contains("> note"), "{all}");
    press(&mut app, "ae more\n");
    let export = app.export();
    assert!(export.contains("> note more"), "an edit makes the annotation unsent again:\n{export}");
}

#[test]
fn send_with_all_sent_says_nothing_new() {
    let mut app = annotated(Agent::ClaudeCode, idle());
    app.finish_send("2026-11-09T11:00:00Z");
    let effect = press_key(&mut app, KeyCode::Char('S'), KeyModifiers::SHIFT);
    assert_eq!(effect, Effect::None);
    assert_eq!(app.status(), Some("Nothing new to send. Press Y to copy all notes."));
}

#[test]
fn older_reply_has_its_own_name_in_the_export() {
    let mut app = three_replies();
    press(&mut app, "cmine\n");
    assert!(app.export().starts_with("# Annotations on last reply"), "{}", app.export());
    press(&mut app, "pj");
    press_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    press(&mut app, "cold\n");
    assert!(app.export().contains("# Annotations on earlier reply (10:30)"), "{}", app.export());
}

#[test]
fn unreadable_draft_is_named_in_the_quit_question() {
    let mut app = annotated(Agent::ClaudeCode, idle());
    app.set_save_error(Some("the draft x.json cannot be read".to_owned()));
    press(&mut app, "q");
    let screen = render(&mut app, 120, 12);
    assert!(screen.contains("The draft cannot be saved: the draft x.json cannot be read"), "{screen}");
    assert!(!screen.contains("k keep the draft"), "{screen}");
    assert_eq!(press_key(&mut app, KeyCode::Char('k'), KeyModifiers::NONE), Effect::None, "k does nothing");
    assert_eq!(press_key(&mut app, KeyCode::Char('d'), KeyModifiers::NONE), Effect::Discard);
}

#[test]
fn x_removes_an_unreadable_draft_of_an_empty_review() {
    let mut app = app("alpha beta\n");
    app.set_save_error(Some("the draft x.json cannot be read".to_owned()));
    press(&mut app, "X");
    assert!(render(&mut app, 120, 12).contains("Remove the draft file that Annorex cannot read? y/n"));
    assert_eq!(press_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE), Effect::Discard);
}

#[test]
fn held_enter_in_the_quit_flow_stays_open() {
    use annorex::deliver::DeliveryMode;
    use annorex::send::{SendResult, apply_result};
    let mut app = annotated(Agent::ClaudeCode, idle());
    press(&mut app, "q");
    assert_eq!(press_key(&mut app, KeyCode::Char('s'), KeyModifiers::NONE), Effect::Send(DeliveryMode::Send));
    apply_result(
        &mut app,
        SendResult::Held("Pasted, but Enter was held: press Enter in the agent pane.".to_owned()),
        "t",
    );
    assert!(!app.should_exit(), "the viewer stays so the message shows");
    assert!(app.status().is_some_and(|text| text.contains("Enter was held")));
}

#[test]
fn ctrl_c_asks_once() {
    let mut app = annotated(Agent::ClaudeCode, idle());
    let ctrl_c = |app: &mut App| press_key(app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(ctrl_c(&mut app), Effect::None);
    assert!(render(&mut app, 120, 12).contains("Unsent notes."));
    assert_eq!(ctrl_c(&mut app), Effect::Quit);
    let mut editing = annotated(Agent::ClaudeCode, idle());
    press(&mut editing, "cabc");
    assert_eq!(press_key(&mut editing, KeyCode::Char('c'), KeyModifiers::CONTROL), Effect::SaveDraft);
    assert!(editing.editor().is_none(), "ctrl+c closes the editor first");
    let mut clean = app_with("alpha\n", &Config::default());
    assert_eq!(press_key(&mut clean, KeyCode::Char('c'), KeyModifiers::CONTROL), Effect::Quit);
}

#[test]
fn cursor_is_visible_on_a_trailing_space() {
    use ratatui::style::Modifier;
    // 40 columns: the margin, 37 columns of text, and the marker margin of 2 columns.
    let text = format!("{} {}\n", "a".repeat(36), "b".repeat(10));
    let mut app = app(&text);
    press(&mut app, "0");
    for _ in 0..36 {
        press(&mut app, "l");
    }
    assert_eq!(app.cursor(), 36, "the cursor is on the space");
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).expect("terminal");
    terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let reversed = (1..40u16)
        .flat_map(|x| (1..4u16).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            buffer.cell((x, y)).expect("cell").style().add_modifier.contains(Modifier::REVERSED)
        })
        .collect::<Vec<_>>();
    assert_eq!(reversed, [(37, 1)], "the cursor shows on the trailing space");
}

#[test]
fn reply_text_change_moves_annotations_and_the_editor_range() {
    let mut app = app("alpha beta\n");
    press(&mut app, "wvlllc");
    press(&mut app, "x\n");
    press(&mut app, "g0vllllcn");
    assert!(app.editor().is_some());
    let new = "intro line\n\nalpha beta\n";
    app.set_replies(vec![reply("r1", new)]);
    let beta = new.find("beta").expect("beta");
    let first = &app.review().expect("review").annotations[0];
    assert_eq!((first.range.clone(), first.detached), (beta..beta + 4, false), "the saved note moved");
    press(&mut app, "\n");
    let alpha = new.find("alpha").expect("alpha");
    let second = &app.review().expect("review").annotations[1];
    assert_eq!(second.range, alpha..alpha + 5, "the note under edit moved");
    assert_eq!(second.quote, "alpha");
}

#[test]
fn an_editor_on_lost_text_closes_with_a_message() {
    let mut app = app("alpha beta\n");
    press(&mut app, "vllllcn");
    app.set_replies(vec![reply("r1", "totally different\n")]);
    assert!(app.editor().is_none(), "the editor closed");
    assert!(app.status().is_some_and(|text| text.contains("is gone")), "{:?}", app.status());
}

#[test]
fn hints_follow_rebound_keys() {
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("config.toml");
    std::fs::write(&file, "[keys]\ncomment = \"y\"\nquit = \"Z\"\nreload = \"R\"\n").expect("config");
    let config = Config::load(&file).expect("config");
    let mut app = app_with("alpha beta\n", &config);
    let screen = render(&mut app, 120, 12);
    let footer = screen.lines().last().expect("footer").to_owned();
    assert!(footer.contains("o y d s note"), "{footer}");
    assert!(!footer.contains("c d s"), "{footer}");
    let mut error =
        App::new(state_of(Agent::ClaudeCode, idle()), Screen::Error("x".to_owned()), Vec::new(), &config);
    let footer = render(&mut error, 120, 12).lines().last().expect("footer").to_owned();
    assert!(footer.contains("R try again") && footer.contains("Z quit"), "{footer}");
}

#[test]
fn selection_gives_the_range_of_the_annotation() {
    let text = "**bold** text\nsecond line\n";
    let mut app = app(text);
    press(&mut app, "0vllllcn\n");
    let first = &app.review().expect("review").annotations[0];
    assert_eq!(first.quote, "bold** ", "a selection over markup quotes the source, markup included");
    assert_eq!(first.range, 2..9);
    let mut lines = self::app(text);
    press(&mut lines, "Vjd");
    let delete = &lines.review().expect("review").annotations[0];
    assert_eq!(delete.range.end, text.len() - 1, "both lines, without the final line feed");
    assert!(delete.quote.ends_with("\nsecond line"), "{:?}", delete.quote);
}

#[test]
fn a_second_identical_note_is_not_added() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllllcWhich page?\n");
    press(&mut app, "hhhhvllllcWhich page?\n");
    assert_eq!(app.review().expect("review").annotations.len(), 1);
    assert!(render(&mut app, 60, 12).contains("Already saved."));
}

#[test]
fn a_note_with_other_text_on_the_same_range_is_added() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllllcWhich page?\n");
    press(&mut app, "hhhhvllllcWhich book?\n");
    assert_eq!(app.review().expect("review").annotations.len(), 2);
    assert!(!render(&mut app, 60, 12).contains("Already saved."));
}

#[test]
fn an_edit_into_a_copy_of_another_note_is_refused() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllllcWhich page?\n");
    press(&mut app, "hhhhvllllcWhich book?\n");
    press(&mut app, "hhhh");
    press(&mut app, "L");
    assert_eq!(app.editor().expect("editor").text, "Which page?");
    for _ in 0.."Which page?".len() {
        press_key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
    }
    press(&mut app, "Which book?\n");
    let notes: Vec<&str> =
        app.review().expect("review").annotations.iter().map(|a| a.note.as_str()).collect();
    assert_eq!(notes, ["Which page?", "Which book?"], "the edited note keeps its old text");
    assert_eq!(app.editor().expect("the editor stays open").text, "Which book?", "the typed text stays");
    assert!(render(&mut app, 60, 12).contains("Another note has this text."));
}

#[test]
fn an_edit_that_keeps_its_own_text_is_saved() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllllcWhich page?\n");
    press(&mut app, "hhhhL\n");
    assert_eq!(app.review().expect("review").annotations.len(), 1);
    assert!(!render(&mut app, 60, 12).contains("Already saved."));
}

#[test]
fn a_change_to_error_closes_the_help_so_the_next_key_acts() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "?");
    assert!(app.help_open);
    app.set_error("the transcript is gone".to_owned());
    assert!(!app.help_open);
    assert_eq!(press_key(&mut app, KeyCode::Char('r'), KeyModifiers::NONE), Effect::Reload);
}

#[test]
fn a_sent_note_can_be_added_again() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllllcWhich page?\n");
    app.finish_send("2026-11-12T10:00:00+07:00");
    press(&mut app, "hhhhvllllcWhich page?\n");
    assert_eq!(app.review().expect("review").annotations.len(), 2);
    assert_eq!(app.unsent_count(), 1);
}

/// The index of the first screen row that holds `needle`.
fn row_of(screen: &str, needle: &str) -> usize {
    screen
        .lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no {needle:?} in\n{screen}"))
}

const BAR: &str =
    "\u{2590} o \u{2713} looks good  c \u{1f4ac} comment  d \u{2715} remove  s \u{21c4} replace \u{258c}";

#[test]
fn the_strip_shows_above_the_block_after_enter_with_live_keys() {
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("config.toml");
    std::fs::write(&file, "[keys]\nlooks_good = \"y\"\n").expect("config");
    let config = Config::load(&file).expect("config");
    let mut app = app_with("# Title\n\nfirst para\n\nsecond para\n", &config);
    press(&mut app, "j\n");
    let screen = render(&mut app, 120, 40);
    let bar = row_of(
        &screen,
        "\u{2590} y \u{2713} looks good  c \u{1f4ac} comment  d \u{2715} remove  s \u{21c4} replace \u{258c}",
    );
    assert_eq!(bar + 1, row_of(&screen, "first para"), "{screen}");
    assert!(!screen.contains("o \u{2713} looks good"), "{screen}");
    let line = screen.lines().nth(bar).expect("bar row");
    assert!(line.starts_with(" \u{2590} y"), "the strip starts at the text column: {line:?}");
}

#[test]
fn the_strip_shows_beside_a_finished_selection_in_text_mode() {
    let mut app = app(LIST);
    // Text mode at the first item, then select the first two lines. The second `V` ends the selection.
    press(&mut app, "\nVj");
    assert!(!render(&mut app, 80, 24).contains(BAR), "no strip while the selection grows");
    press(&mut app, "V");
    let screen = render(&mut app, 80, 24);
    // The selection starts on the first row: the strip goes below it.
    assert_eq!(row_of(&screen, BAR), row_of(&screen, "two") + 1, "{screen}");
    assert!(!screen.contains("three"), "the strip lies over the next row: {screen}");
    let mut chars = self::app("first line\n\nalpha beta gamma\n");
    press(&mut chars, "jwvllv");
    let screen = render(&mut chars, 80, 24);
    assert_eq!(row_of(&screen, BAR) + 1, row_of(&screen, "alpha"), "{screen}");
    let line = screen.lines().find(|line| line.contains("alpha")).expect("alpha");
    let strip = screen.lines().nth(row_of(&screen, BAR)).expect("strip");
    assert_eq!(strip.find('\u{2590}'), line.find("beta"), "the strip starts at the selection:\n{screen}");
}

#[test]
fn no_bar_while_the_editor_is_open() {
    let mut app = app("alpha beta\n\nsecond\n");
    press(&mut app, "\n");
    assert!(render(&mut app, 80, 24).contains(BAR));
    press(&mut app, "c");
    assert!(!render(&mut app, 80, 24).contains("comment  d"), "no strip with the editor");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!render(&mut app, 80, 24).contains(BAR), "the editor closed the strip");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, "\n?");
    assert!(!render(&mut app, 80, 24).contains(BAR), "no strip under the help screen");
    let mut empty = self::app("");
    press(&mut empty, "\n");
    assert!(!render(&mut empty, 80, 24).contains(BAR), "no strip for an empty document");
}

/// Thirty paragraphs: more than one screen of 80x24.
fn many_paragraphs() -> String {
    (1..=30).map(|n| format!("paragraph {n}\n\n")).collect::<Vec<_>>().concat()
}

/// The rows of the top and the bottom border of the editor box.
fn editor_rows(screen: &str) -> (usize, usize) {
    let top = screen
        .lines()
        .position(|line| line.contains('┌') && line.contains("Comment"))
        .unwrap_or_else(|| panic!("no editor in\n{screen}"));
    let bottom = row_of(screen, " save ");
    (top, bottom)
}

#[test]
fn the_editor_opens_below_the_block_or_above_at_the_bottom() {
    let mut app = app(&many_paragraphs());
    press(&mut app, "jc");
    let screen = render(&mut app, 80, 24);
    let (top, bottom) = editor_rows(&screen);
    assert_eq!(top, row_of(&screen, "paragraph 2") + 1, "below the block:\n{screen}");
    assert!(screen.lines().nth(bottom).is_some_and(|line| line.contains(" cancel ")), "{screen}");
    let box_line = screen.lines().nth(top).expect("top");
    assert!(box_line.starts_with(" ┌"), "the box starts at the text column: {box_line:?}");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, "Gc");
    let screen = render(&mut app, 80, 24);
    let (_, bottom) = editor_rows(&screen);
    assert_eq!(
        bottom + 1,
        row_of(&screen, "\u{258d}paragraph 30"),
        "above the block at the bottom:\n{screen}"
    );
}

#[test]
fn a_resize_keeps_the_focused_block_and_the_editor_text() {
    let mut app = app(&many_paragraphs());
    render(&mut app, 120, 40);
    press(&mut app, "jjc");
    press(&mut app, "half a note");
    render(&mut app, 120, 40);
    let screen = render(&mut app, 60, 20);
    assert_eq!(app.focused_block(), Some(2));
    assert_eq!(app.editor().expect("editor").text, "half a note");
    assert!(screen.contains("half a note▏"), "{screen}");
    let (top, _) = editor_rows(&screen);
    assert_eq!(top, row_of(&screen, "paragraph 3") + 1, "{screen}");
}

fn bar_text() -> &'static str {
    "# Plan\n\nWe change the login page.\nThe old page stays for a week.\n\n- one\n- two\n\nThe end.\n"
}

#[test]
fn snapshot_action_bar_120x40() {
    let mut app = app(bar_text());
    press(&mut app, "j\n");
    insta::assert_snapshot!(render(&mut app, 120, 40));
}

#[test]
fn snapshot_editor_floating_80x24() {
    let mut app = app(bar_text());
    press(&mut app, "jc");
    press(&mut app, "Which page?");
    insta::assert_snapshot!(render(&mut app, 80, 24));
}

/// Four paragraphs and a last one, so the focus can leave the notes.
const FIVE: &str = "good part\n\nasked part\n\ngone part\n\nold part\n\nthe end\n";

/// One note of each kind: looks good, comment, remove, replace. The focus ends on the last block.
fn four_kinds() -> App {
    let mut app = app(FIVE);
    press(&mut app, "o");
    press(&mut app, "jcWhy?\n");
    press(&mut app, "jd");
    press(&mut app, "jsnew part\n");
    press(&mut app, "j");
    assert_eq!(app.review().expect("review").annotations.len(), 4);
    app
}

#[test]
fn range_styles_follow_the_kind() {
    use ratatui::style::{Color, Modifier};
    let mut app = four_kinds();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let screen = render(&mut app, 80, 24);
    let cell = |text: &str| {
        let y = u16::try_from(row_of(&screen, text)).expect("row");
        buffer.cell((2, y)).expect("cell").style()
    };
    let good = cell("good part");
    assert_eq!(good.bg, Some(Color::Green), "looks good: green background");
    let asked = cell("asked part");
    assert_eq!(asked.fg, Some(Color::Yellow), "comment: yellow");
    assert!(asked.add_modifier.contains(Modifier::UNDERLINED), "comment: underlined");
    let gone = cell("gone part");
    assert_eq!(gone.fg, Some(Color::Red), "remove: red");
    assert!(gone.add_modifier.contains(Modifier::CROSSED_OUT), "remove: crossed out");
    let old = cell("old part");
    assert_eq!(old.fg, Some(Color::Magenta), "replace: magenta");
    assert!(old.add_modifier.contains(Modifier::CROSSED_OUT), "replace: crossed out");
    let end = cell("the end");
    assert!(!end.add_modifier.contains(Modifier::UNDERLINED | Modifier::CROSSED_OUT), "no note: no style");
}

/// The cells of the screen as a grid of symbols and styles.
fn drawn(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    terminal.backend().buffer().clone()
}

/// The cells that show `symbol`, as (column, row, style).
fn cells_with(buffer: &ratatui::buffer::Buffer, symbol: &str) -> Vec<(u16, u16, ratatui::style::Style)> {
    let area = buffer.area;
    let mut out = Vec::new();
    for y in 0..area.height {
        for x in 0..area.width {
            let cell = buffer.cell((x, y)).expect("cell");
            if cell.symbol() == symbol {
                out.push((x, y, cell.style()));
            }
        }
    }
    out
}

#[test]
fn cards_show_at_120_columns_and_markers_below() {
    use ratatui::style::Color;
    let mut app = app("# Plan\n\nWe change the login page.\n\nThe end.\n");
    press(&mut app, "jcWhich page?\n");
    let wide = render(&mut app, 120, 40);
    let title = row_of(&wide, "💬 comment");
    assert_eq!(title, row_of(&wide, "We change the login page."), "the card is at its block:\n{wide}");
    assert!(wide.contains("Which page?"), "{wide}");
    let line = wide.lines().nth(title).expect("title row");
    let card_x = line.find('┌').expect("card border");
    assert!(card_x > 80, "the card is in the rail on the right: {line:?}");
    let buffer = drawn(&mut app, 120, 40);
    let corners = cells_with(&buffer, "┌");
    assert!(corners.iter().any(|(_, _, style)| style.fg == Some(Color::Yellow)), "the border is yellow");
    assert!(cells_with(&buffer, "●").is_empty(), "no marker with the rail");

    let narrow = render(&mut app, 110, 40);
    assert!(!narrow.contains("💬 comment"), "no card below 120 columns:\n{narrow}");
    let buffer = drawn(&mut app, 110, 40);
    let markers = cells_with(&buffer, "●");
    let row = u16::try_from(row_of(&narrow, "We change the login page.")).expect("row");
    assert_eq!(markers.len(), 1, "{narrow}");
    let (x, y, style) = markers[0];
    assert_eq!((y, style.fg), (row, Some(Color::Yellow)), "a yellow marker at the row of the note");
    assert!(x > 100, "the marker is in the right margin: {x}");
}

#[test]
fn e_edits_the_note_of_the_focused_block() {
    let mut app = app("# Plan\n\nWe change the login page.\n\nThe end.\n");
    press(&mut app, "jcfirst\n");
    press(&mut app, "k");
    press(&mut app, "e");
    assert!(app.editor().is_none(), "the title has no note");
    assert!(render(&mut app, 80, 24).contains("no note here"));
    press(&mut app, "je");
    assert_eq!(app.editor().expect("editor").text, "first");
    press(&mut app, "!\n");
    let review = app.review().expect("review");
    assert_eq!((review.annotations.len(), review.annotations[0].note.as_str()), (1, "first!"));
}

#[test]
fn e_follows_a_rebound_key() {
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("config.toml");
    std::fs::write(&file, "[keys]\nedit = \"E\"\n").expect("config");
    let config = Config::load(&file).expect("config");
    let mut app = app_with("We change the login page.\n", &config);
    press(&mut app, "cfirst\n");
    press(&mut app, "E");
    assert_eq!(app.editor().expect("editor").text, "first");
}

#[test]
fn x_asks_then_removes_it() {
    let mut app = app("# Plan\n\nWe change the login page.\n\nThe end.\n");
    press(&mut app, "jcfirst\n");
    press(&mut app, "x");
    assert!(render(&mut app, 80, 24).contains("Delete annotation 1? y/n"));
    press(&mut app, "n");
    assert_eq!(app.review().expect("review").annotations.len(), 1, "n keeps it");
    press(&mut app, "xy");
    assert!(app.review().expect("review").is_empty(), "y removes it");
}

#[test]
fn several_notes_show_a_numbered_pick() {
    let mut app = app("# Plan\n\nWe change the login page.\n\nThe end.\n");
    press(&mut app, "jcone\n");
    press(&mut app, "ctwo\n");
    press(&mut app, "e");
    assert!(app.editor().is_none(), "two notes: no editor yet");
    let screen = render(&mut app, 80, 24);
    let first = row_of(&screen, "1 💬 one");
    assert_eq!(row_of(&screen, "2 💬 two"), first + 1, "{screen}");
    assert!(!screen.contains("c comment"), "no action bar under the pick:\n{screen}");
    press(&mut app, "2");
    assert_eq!(app.editor().expect("editor").text, "two");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    press(&mut app, "e");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.editor().is_none(), "esc closes the pick");
    assert!(!render(&mut app, 80, 24).contains("1 💬 one"));

    press(&mut app, "x9");
    assert!(app.editor().is_none() && !render(&mut app, 80, 24).contains("y/n"), "9 is not in the list");
    press(&mut app, "1");
    assert!(render(&mut app, 80, 24).contains("Delete annotation 1? y/n"));
    press(&mut app, "y");
    let review = app.review().expect("review");
    assert_eq!((review.annotations.len(), review.annotations[0].note.as_str()), (1, "two"));
}

#[test]
fn snapshot_cards_120x40() {
    let mut app = app(bar_text());
    press(&mut app, "o");
    press(&mut app, "jcWhich page? The old one or the new one, after the move of the login form.\n");
    press(&mut app, "jd");
    press(&mut app, "jsThe last line.\n");
    press(&mut app, "k");
    insta::assert_snapshot!(render(&mut app, 120, 40));
}

#[test]
fn a_card_starts_on_the_first_row_of_its_block_at_240_columns() {
    let mut app = app(bar_text());
    // The blocks are the heading, the paragraph, "- one", "- two", and "The end.": block 3 is "- two".
    press(&mut app, "jjjcNote on two\n");
    let screen = render(&mut app, 240, 40);
    let block = row_of(&screen, "- two");
    let title = row_of(&screen, "💬 comment");
    assert_eq!(title, block, "the card top is the first row of its block:\n{screen}");
    // The card sits in the rail at the right edge of the window.
    let line = screen.lines().nth(title).expect("row");
    let card_x = line.find('┌').expect("card border");
    assert!((200..210).contains(&card_x), "the card is in the rail at the right edge: {card_x}");
}

#[test]
fn snapshot_wide_200x40() {
    let mut app = app(bar_text());
    press(&mut app, "jcWhich page? The old one or the new one.\n");
    insta::assert_snapshot!(render(&mut app, 200, 40));
}

#[test]
fn the_open_list_keeps_the_text_width_at_119_columns() {
    let mut app = app(bar_text());
    press(&mut app, "a");
    let screen = render(&mut app, 119, 40);
    // No rail and no cards: the list takes 34 columns at the right edge, the document keeps 85.
    let line = screen.lines().nth(2).expect("row");
    let border = line.chars().take_while(|&c| c != '│').count();
    assert_eq!(border, 85, "{screen}");
}

#[test]
fn d_saves_remove_at_once() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllll");
    let effect = press_key(&mut app, KeyCode::Char('d'), KeyModifiers::NONE);
    assert!(matches!(effect, Effect::SaveDraft));
    assert!(app.editor().is_none(), "no box opens");
    let review = app.review().expect("review");
    assert_eq!(review.annotations.len(), 1);
    assert_eq!((review.annotations[0].kind, review.annotations[0].quote.as_str()), (Kind::Delete, "alpha"));
    press(&mut app, "0vlllld");
    assert_eq!(app.review().expect("review").annotations.len(), 1);
    assert_eq!(app.status(), Some("Already saved."));
}

#[test]
fn looks_good_and_remove_cards_have_no_edit() {
    for key in ["o", "d"] {
        let mut app = app("alpha beta gamma\n");
        press(&mut app, "vllll");
        press(&mut app, key);
        let screen = render(&mut app, 140, 24);
        assert!(screen.contains("x remove"), "{key}:\n{screen}");
        assert!(!screen.contains("e edit"), "{key}:\n{screen}");
    }
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllllc");
    press(&mut app, "Why?\n");
    let screen = render(&mut app, 140, 24);
    assert!(screen.contains("e edit") && screen.contains("x remove"), "{screen}");
}

#[test]
fn e_on_looks_good_does_not_open_box() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vlllloe");
    assert!(app.editor().is_none());
    assert_eq!(app.status(), Some("Looks good has no text."));
    let mut app = self::app("alpha beta gamma\n");
    press(&mut app, "vllllde");
    assert!(app.editor().is_none());
    assert_eq!(app.status(), Some("Remove has no text."));
}

#[test]
fn an_old_looks_good_note_with_text_still_exports_its_text() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vlllld");
    let mut draft = app.draft(Agent::ClaudeCode, "s").expect("draft");
    draft.review.annotations[0].note = "old text".to_owned();
    let mut fresh = self::app("alpha beta gamma\n");
    fresh.restore("r1", draft);
    let export = fresh.export();
    assert!(export.contains("> old text"), "{export}");
}

#[test]
fn an_old_o_looks_good_note_with_text_still_exports_its_text() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllllo");
    let mut draft = app.draft(Agent::ClaudeCode, "s").expect("draft");
    assert_eq!(draft.review.annotations[0].kind, Kind::LooksGood);
    draft.review.annotations[0].note = "old good text".to_owned();
    let mut fresh = self::app("alpha beta gamma\n");
    fresh.restore("r1", draft);
    let export = fresh.export();
    assert!(export.contains("> old good text"), "{export}");
}

#[test]
fn esc_with_nothing_selected_quits() {
    let mut app = app("alpha beta gamma\n");
    // The first esc removes the block focus. The second one quits.
    assert_eq!(press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE), Effect::None);
    assert_eq!(press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE), Effect::Quit);
}

#[test]
fn esc_with_focus_clears_it() {
    let mut app = app("alpha beta gamma\n");
    assert_ne!(press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE), Effect::Quit);
    assert!(!render(&mut app, 80, 12).contains("esc unselect"));
    // A text selection: the first esc clears it, the second leaves text mode, the third removes the focus.
    let mut text = self::app("alpha beta gamma\n");
    press(&mut text, "vll");
    for _ in 0..3 {
        assert_eq!(press_key(&mut text, KeyCode::Esc, KeyModifiers::NONE), Effect::None);
    }
    assert_eq!(press_key(&mut text, KeyCode::Esc, KeyModifiers::NONE), Effect::Quit);
}

#[test]
fn esc_with_unsent_notes_asks_first() {
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllllo");
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    let mut guard = 0;
    while !render(&mut app, 120, 12).contains("Unsent notes") && guard < 5 {
        assert_ne!(press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE), Effect::Quit, "no quit at once");
        guard += 1;
    }
    assert!(render(&mut app, 120, 12).contains("Unsent notes"));
    assert!(!app.should_exit());
}

#[test]
fn footer_always_ends_with_q_quit() {
    let mut app = app("alpha beta gamma\n");
    for width in [80, 120] {
        let screen = render(&mut app, width, 12);
        let footer = screen.lines().last().expect("footer");
        assert!(footer.ends_with("q quit"), "{width}: {footer}");
        assert!(footer.contains("? help"), "{width}: {footer}");
    }
    press(&mut app, "vl");
    let footer_of = |app: &mut App| render(app, 80, 12).lines().last().expect("footer").to_owned();
    assert!(footer_of(&mut app).ends_with("q quit"), "{}", footer_of(&mut app));
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(footer_of(&mut app).ends_with("q quit"), "{}", footer_of(&mut app));
}

#[test]
fn footer_shows_esc_unselect_only_with_focus() {
    let mut app = app("alpha beta gamma\n");
    assert!(render(&mut app, 120, 12).contains("esc unselect"));
    press_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!render(&mut app, 120, 12).contains("esc unselect"));
    press(&mut app, "vl");
    assert!(render(&mut app, 120, 12).contains("esc unselect"));
}

#[test]
fn successful_send_quits() {
    use annorex::send::{SendResult, apply_result};
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllllo");
    assert!(!app.should_exit());
    assert!(apply_result(&mut app, SendResult::Sent, "2026-11-09T14:30:00Z"));
    assert!(app.draft(Agent::ClaudeCode, "s").expect("draft").sent_at.is_some(), "marked sent first");
    assert!(app.should_exit());
}

#[test]
fn failed_send_stays_open() {
    use annorex::send::{SendResult, apply_result};
    let mut app = app("alpha beta gamma\n");
    press(&mut app, "vllllo");
    apply_result(&mut app, SendResult::Refused("Not sent: the agent shows a dialog.".to_owned()), "t");
    assert!(!app.should_exit());
    assert!(render(&mut app, 120, 12).contains("Not sent: the agent shows a dialog."));
}
