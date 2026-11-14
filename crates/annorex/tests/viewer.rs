//! The viewer: layout at fixed sizes (snapshots), resizes, find, the reply list, and keys.

#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use std::path::PathBuf;
use std::time::SystemTime;

use annorex::agent::Agent;
use annorex::config::Config;
use annorex::replies::{Candidate, Reply, SessionRef};
use annorex::safety::Safety;
use annorex::ui::{AgentState, App, Effect, Screen, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Modifier;
use unicode_width::UnicodeWidthStr;

const LONG: &str = include_str!("fixtures/reply-long.md");

fn state(agent: Agent, state: &str) -> AgentState {
    AgentState { agent: Some(agent), safety: Safety::Clear { state: Some(state.to_owned()) } }
}

fn reply(id: &str, text: &str) -> Reply {
    Reply { id: id.to_owned(), text: text.to_owned(), at: Some("2026-11-09T10:30:30.000Z".to_owned()) }
}

fn replies() -> Vec<Reply> {
    vec![
        reply("r1", LONG),
        reply("r2", "## Earlier answer\n\nThe older reply."),
        reply("r3", "Oldest reply: short."),
    ]
}

fn viewer() -> App {
    let mut app = App::new(state(Agent::ClaudeCode, "idle"), Screen::Viewer, replies(), &Config::default());
    // The snapshots must not depend on the time zone of the machine.
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    // The view starts without a focus: `j` focuses the first block.
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    app
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn press(app: &mut App, keys: &str) -> Effect {
    let mut effect = Effect::None;
    for c in keys.chars() {
        let code = match c {
            '\n' => KeyCode::Enter,
            '\u{1b}' => KeyCode::Esc,
            c => KeyCode::Char(c),
        };
        effect = app.handle_key(key(code));
    }
    effect
}

/// Draw the app at `width` × `height` and return the screen as lines of text.
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
            // A wide character covers the next cells too. Do not print them.
            x += u16::try_from(symbol.width().max(1)).unwrap_or(1);
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}

/// The text of the first document row (row 1), without the margin.
fn first_row_text(screen: &str) -> String {
    screen.lines().nth(1).unwrap_or_default().trim_start_matches(['\u{258d}', ' ']).trim_end().to_owned()
}

/// True when the first document row shows the start of the display line `index`.
fn top_shows(app: &App, screen: &str, index: usize) -> bool {
    let row = first_row_text(screen);
    let line = app.doc().lines[index].plain();
    !row.is_empty() && line.starts_with(&row)
}

fn first_text_line_at_or_after(app: &App, index: usize) -> usize {
    (index..app.doc().lines.len()).find(|&i| !app.doc().lines[i].is_blank()).expect("a text line")
}

#[test]
fn snapshot_40x12() {
    insta::assert_snapshot!(render(&mut viewer(), 40, 12));
}

#[test]
fn snapshot_80x24() {
    insta::assert_snapshot!(render(&mut viewer(), 80, 24));
}

#[test]
fn snapshot_120x40() {
    insta::assert_snapshot!(render(&mut viewer(), 120, 40));
}

#[test]
fn snapshot_218x69() {
    insta::assert_snapshot!(render(&mut viewer(), 218, 69));
}

#[test]
fn snapshot_39x9_is_too_small() {
    let screen = render(&mut viewer(), 39, 9);
    assert!(screen.contains("pane too small"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn the_40_column_header_keeps_the_agent_state() {
    let screen = render(&mut viewer(), 40, 12);
    let header = screen.lines().next().expect("header");
    assert!(header.contains("claude ("), "{header}");
    assert!(header.contains("reply 1/3"), "{header}");
}

#[test]
fn header_names_view_agent_state_and_reply() {
    let screen = render(&mut viewer(), 120, 40);
    let header = screen.lines().next().expect("header");
    assert!(header.starts_with("Annorex · last reply · Claude Code (idle)"), "{header}");
    assert!(header.contains("Claude Code (idle) · reply 1/3"), "{header}");
    assert!(header.ends_with("[ Send ▸ ]"), "{header}");
}

#[test]
fn resize_sequence_keeps_top_line() {
    let mut app = viewer();
    let index = first_text_line_at_or_after(&app, 30);
    app.jump_to_line(index);
    for (width, height) in [(218, 69), (40, 12), (120, 40), (80, 24)] {
        let screen = render(&mut app, width, height);
        assert!(top_shows(&app, &screen, index), "at {width}×{height}:\n{screen}");
    }
}

#[test]
fn resize_below_minimum_and_back_keeps_top_line() {
    let mut app = viewer();
    let index = first_text_line_at_or_after(&app, 30);
    app.jump_to_line(index);
    let screen = render(&mut app, 218, 69);
    assert!(top_shows(&app, &screen, index), "{screen}");
    assert!(render(&mut app, 30, 8).contains("pane too small"));
    let screen = render(&mut app, 218, 69);
    assert!(top_shows(&app, &screen, index), "{screen}");
}

#[test]
fn cursor_line_stays_visible_when_moving_down() {
    let mut app = viewer();
    render(&mut app, 80, 24);
    for _ in 0..60 {
        press(&mut app, "j");
        render(&mut app, 80, 24);
    }
    let cursor = app.cursor_line();
    let screen = render(&mut app, 80, 24);
    let start: String = app.doc().lines[cursor].plain().chars().take(12).collect();
    assert!(!start.trim().is_empty(), "the check needs text on the cursor line");
    let doc_rows: Vec<&str> = screen.lines().skip(1).take(22).collect();
    assert!(doc_rows.iter().any(|row| row.contains(&start)), "{screen}");
}

#[test]
fn find_moves_cursor_and_wraps() {
    let mut app = viewer();
    render(&mut app, 120, 40);
    press(&mut app, "/note 2:\n");
    let first = app.cursor_line();
    assert!(app.doc().lines[first].plain().starts_with("Note 2:"), "{}", app.doc().lines[first].plain());
    press(&mut app, "/table\n");
    let table = app.cursor_line();
    assert_eq!(app.doc().lines[table].plain(), "Table");
    press(&mut app, "/oldest\u{1b}");
    assert_eq!(app.cursor_line(), table, "esc cancels the find");
    press(&mut app, "/migration\n");
    assert_eq!(app.cursor_line(), 0, "the search wraps to the start");
    press(&mut app, "n");
    assert_eq!(app.cursor_line(), 0, "one match: n stays on it");
    press(&mut app, "/the text of\n");
    let a = app.cursor_line();
    press(&mut app, "n");
    let b = app.cursor_line();
    press(&mut app, "N");
    assert!(b > a);
    assert_eq!(app.cursor_line(), a);
}

#[test]
fn find_without_match_says_no_match() {
    let mut app = viewer();
    press(&mut app, "/zzzz-nothing\n");
    let screen = render(&mut app, 80, 24);
    assert!(screen.lines().last().expect("footer").contains("no match"), "{screen}");
}

#[test]
fn panel_lists_replies_and_enter_selects() {
    for (width, height) in [(120, 40), (80, 24), (50, 20)] {
        let mut app = viewer();
        press(&mut app, "p");
        let screen = render(&mut app, width, height);
        assert!(screen.contains("Earlier answer"), "at {width}×{height}:\n{screen}");
        assert!(screen.contains("Oldest reply: short."), "at {width}×{height}:\n{screen}");
        press(&mut app, "j\n");
        let screen = render(&mut app, width, height);
        assert!(screen.lines().next().expect("header").contains("reply 2/3"), "{screen}");
        assert!(screen.contains("The older reply."), "at {width}×{height}:\n{screen}");
    }
}

#[test]
fn header_shows_blocked_kind() {
    let mut app = viewer();
    app.set_state(AgentState {
        agent: Some(Agent::OpenCode),
        safety: Safety::Blocked { kind: Some("permission".into()), msg: Some("allow bash?".into()) },
    });
    let header = render(&mut app, 120, 40).lines().next().expect("header").to_owned();
    assert!(header.contains("OpenCode (waiting for permission)"), "{header}");
    app.set_state(AgentState {
        agent: Some(Agent::OpenCode),
        safety: Safety::Blocked { kind: None, msg: None },
    });
    let header = render(&mut app, 120, 40).lines().next().expect("header").to_owned();
    assert!(header.contains("OpenCode (blocked)"), "{header}");
}

#[test]
fn header_shows_working() {
    let mut app = viewer();
    app.set_state(state(Agent::Pi, "working"));
    let header = render(&mut app, 120, 40).lines().next().expect("header").to_owned();
    assert!(header.contains("Pi (working…)"), "{header}");
    app.set_state(AgentState { agent: Some(Agent::Pi), safety: Safety::Clear { state: None } });
    let header = render(&mut app, 120, 40).lines().next().expect("header").to_owned();
    assert!(header.contains("Pi (unknown)"), "{header}");
}

#[test]
fn error_screen_retries_on_r() {
    let mut app = App::new(
        state(Agent::Pi, "idle"),
        Screen::Error("the agent has no reply yet; press r to read the transcript again".into()),
        Vec::new(),
        &Config::default(),
    );
    let screen = render(&mut app, 80, 24);
    assert!(screen.contains("the agent has no reply yet"), "{screen}");
    assert_eq!(press(&mut app, "r"), Effect::Reload);
    app.set_replies(replies());
    assert!(matches!(app.screen, Screen::Viewer));
    assert!(render(&mut app, 80, 24).contains("Migration plan"));
}

#[test]
fn picker_enter_picks_session() {
    let candidate = |name: &str, line: &str| Candidate {
        session: SessionRef::Pi(PathBuf::from(name)),
        updated: SystemTime::now(),
        last_line: line.to_owned(),
    };
    let mut app = App::new(
        state(Agent::Pi, "idle"),
        Screen::Picker(vec![candidate("/a.jsonl", "first session"), candidate("/b.jsonl", "second session")]),
        Vec::new(),
        &Config::default(),
    );
    let screen = render(&mut app, 80, 24);
    assert!(screen.contains("first session") && screen.contains("second session"), "{screen}");
    assert_eq!(press(&mut app, "j\n"), Effect::PickSession(1));
}

#[test]
fn z_does_nothing_because_the_viewer_covers_the_window() {
    assert_eq!(press(&mut viewer(), "z"), Effect::None);
}

#[test]
fn q_and_ctrl_c_quit() {
    assert_eq!(press(&mut viewer(), "q"), Effect::Quit);
    let mut app = viewer();
    assert_eq!(app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), Effect::Quit);
    press(&mut app, "/");
    assert_eq!(press(&mut app, "q"), Effect::None, "q is text in the find input");
}

#[test]
fn reload_keeps_the_selected_reply() {
    let mut app = viewer();
    press(&mut app, "pj\n");
    assert_eq!(press(&mut app, "r"), Effect::Reload);
    app.set_replies(replies());
    let header = render(&mut app, 120, 40).lines().next().expect("header").to_owned();
    assert!(header.contains("reply 2/3"), "{header}");
}

#[test]
fn snapshot_panel_side_120x40() {
    let mut app = viewer();
    press(&mut app, "p");
    insta::assert_snapshot!(render(&mut app, 120, 40));
}

#[test]
fn snapshot_panel_overlay_50x20() {
    let mut app = viewer();
    press(&mut app, "p");
    insta::assert_snapshot!(render(&mut app, 50, 20));
}

#[test]
fn reload_shows_a_new_newest_reply() {
    let mut app = viewer();
    let mut newer = replies();
    newer.insert(0, reply("r0", "## Newest answer\n\nThe new text."));
    app.set_replies(newer);
    let screen = render(&mut app, 120, 40);
    assert!(screen.lines().next().expect("header").contains("reply 1/4"), "{screen}");
    assert!(screen.contains("Newest answer"), "{screen}");
}

#[test]
fn reload_without_new_reply_keeps_the_position() {
    let mut app = viewer();
    let index = first_text_line_at_or_after(&app, 30);
    app.jump_to_line(index);
    render(&mut app, 120, 40);
    app.set_replies(replies());
    let screen = render(&mut app, 120, 40);
    assert!(top_shows(&app, &screen, index), "{screen}");
}

#[test]
fn picker_keeps_the_selected_session_visible() {
    let candidates: Vec<Candidate> = (0..30)
        .map(|n| Candidate {
            session: SessionRef::Pi(PathBuf::from(format!("/s{n}.jsonl"))),
            updated: SystemTime::now(),
            last_line: format!("session number {n:02}"),
        })
        .collect();
    let mut app =
        App::new(state(Agent::Pi, "idle"), Screen::Picker(candidates), Vec::new(), &Config::default());
    for n in 1..30 {
        press(&mut app, "j");
        let screen = render(&mut app, 80, 24);
        let wanted = format!("▸ updated just now · session number {n:02}");
        assert!(screen.contains(&wanted), "after {n} presses:\n{screen}");
    }
    assert_eq!(press(&mut app, "\n"), Effect::PickSession(29));
}

#[test]
fn not_an_agent_shows_in_the_pane() {
    let message = "the focused pane is not running Claude Code, Pi, or OpenCode";
    let state = AgentState { agent: None, safety: Safety::Clear { state: None } };
    let mut app = App::new(state, Screen::Error(message.into()), Vec::new(), &Config::default());
    let screen = render(&mut app, 100, 12);
    let header = screen.lines().next().expect("header");
    assert!(header.contains("no agent"), "{header}");
    assert!(screen.contains(&format!("Annorex: {message}")), "{screen}");
    assert_eq!(press(&mut app, "r"), Effect::Reload);
    assert_eq!(press(&mut app, "q"), Effect::Quit);
}

#[test]
fn wide_state_keeps_reply_position() {
    let mut app = App::new(
        AgentState {
            agent: Some(Agent::Pi), safety: Safety::Clear { state: Some("許可許可許可".into()) }
        },
        Screen::Viewer,
        replies(),
        &Config::default(),
    );
    let screen = render(&mut app, 50, 12);
    let header = screen.lines().next().expect("header");
    assert!(header.contains("reply 1/3") || header.contains('…'), "{header}");
    assert!(header.width() <= 50, "{header}");
    // At a larger width the whole header fits, with the send button at the right edge.
    let header = render(&mut app, 80, 12).lines().next().expect("header").to_owned();
    assert!(header.contains("reply 1/3") && header.ends_with("[ Send ▸ ]"), "{header}");
}

fn candidate(name: &str) -> Candidate {
    Candidate {
        session: SessionRef::Pi(PathBuf::from(format!("/{name}.jsonl"))),
        updated: SystemTime::now(),
        last_line: name.to_owned(),
    }
}

#[test]
fn new_picker_starts_at_the_first_session() {
    let five: Vec<Candidate> = (0..5).map(|n| candidate(&format!("s{n}"))).collect();
    let mut app = App::new(state(Agent::Pi, "idle"), Screen::Picker(five), Vec::new(), &Config::default());
    press(&mut app, "jjjj");
    app.set_picker(vec![candidate("a"), candidate("b")]);
    assert_eq!(press(&mut app, "\n"), Effect::PickSession(0));
    assert!(render(&mut app, 80, 24).contains("▸ updated just now · a"));
}

#[test]
fn a_change_to_the_picker_closes_the_help() {
    let mut app = viewer();
    press(&mut app, "?");
    assert!(app.help_open);
    app.set_picker(vec![candidate("a"), candidate("b")]);
    assert!(!app.help_open);
    assert_eq!(press(&mut app, "\n"), Effect::PickSession(0));
}

#[test]
fn ctrl_keys_do_not_type_in_find() {
    let mut app = viewer();
    press(&mut app, "/ab");
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    let screen = render(&mut app, 80, 24);
    let footer = screen.lines().last().expect("footer");
    assert_eq!(footer.trim(), "/ab", "{footer}");
}

#[test]
fn match_split_by_wrap_is_highlighted() {
    use ratatui::style::Color;
    let text = "alpha beta gamma delta epsil pi the text of end";
    let state = state(Agent::ClaudeCode, "idle");
    let mut app = App::new(state, Screen::Viewer, vec![reply("r1", text)], &Config::default());
    press(&mut app, "/the text\n");
    // The find puts the cursor on the match. Move it to the line start, so no cursor cell hides a match cell.
    press(&mut app, "0");
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).expect("terminal");
    terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let marked = |y: u16| -> String {
        (0..40)
            .filter(|&x| buffer.cell((x, y)).is_some_and(|cell| cell.bg == Color::Yellow))
            .filter_map(|x| buffer.cell((x, y)).map(|cell| cell.symbol().to_owned()))
            .collect()
    };
    assert_eq!(marked(1), "the ", "the first row ends with the start of the match and the space");
    assert_eq!(marked(2), "text", "the second row starts with the rest of the match");
}

fn small(text: &str) -> App {
    let mut app = App::new(
        state(Agent::ClaudeCode, "idle"),
        Screen::Viewer,
        vec![reply("r1", text)],
        &Config::default(),
    );
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    app
}

#[test]
fn motions_follow_the_display() {
    let mut app = small("alpha beta\n\ngamma delta\n");
    assert_eq!(app.cursor(), 0);
    press(&mut app, "lll");
    assert_eq!(app.cursor(), 3);
    press(&mut app, "w");
    assert_eq!(app.cursor(), 6);
    press(&mut app, "$");
    assert_eq!(app.cursor(), 9);
    press(&mut app, "0");
    assert_eq!(app.cursor(), 0);
    press(&mut app, "lllj");
    assert_eq!(app.cursor(), 15, "j keeps the display column");
    press(&mut app, "b");
    assert_eq!(app.cursor(), 12, "b goes to the start of the word");
    press(&mut app, "b");
    assert_eq!(app.cursor(), 6, "b crosses to the word before");
    press(&mut app, "k");
    assert_eq!(app.cursor(), 6);
    press(&mut app, "hhhhhhhhhhhh");
    assert_eq!(app.cursor(), 0, "h stops at the first stop of the line");
}

#[test]
fn l_stops_at_the_last_char_and_h_at_the_first() {
    let mut app = small("ab\n");
    press(&mut app, "lllll");
    assert_eq!(app.cursor(), 1);
    press(&mut app, "hhhh");
    assert_eq!(app.cursor(), 0);
}

#[test]
fn g_and_capital_g_go_to_the_first_and_last_line() {
    let mut app = small("one\n\ntwo\n");
    // Without a focus, `g` and `G` only scroll: `j` focuses the first block first.
    press(&mut app, "jG");
    assert_eq!(app.cursor(), 5);
    press(&mut app, "g");
    assert_eq!(app.cursor(), 0);
}

#[test]
fn cursor_cell_is_reversed() {
    let mut app = small("alpha beta\n");
    press(&mut app, "l");
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).expect("terminal");
    terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let reversed =
        |x: u16| buffer.cell((x, 1)).expect("cell").style().add_modifier.contains(Modifier::REVERSED);
    assert_eq!(buffer.cell((2, 1)).expect("cell").symbol(), "l");
    assert!(reversed(2), "the second character is the cursor");
    assert!(!reversed(1), "the first character is not");
    assert!(!reversed(3));
}

#[test]
fn the_session_picker_is_a_centered_box() {
    let mut app = App::new(
        state(Agent::Pi, "idle"),
        Screen::Picker(vec![candidate("a"), candidate("b")]),
        Vec::new(),
        &Config::default(),
    );
    let screen = render(&mut app, 80, 24);
    let lines: Vec<Vec<char>> = screen.lines().map(|line| line.chars().collect()).collect();
    let y =
        lines.iter().position(|line| line.iter().collect::<String>().contains("┌ Sessions")).expect("box");
    let x = lines[y].iter().position(|&c| c == '┌').expect("corner");
    let right = lines[y].iter().position(|&c| c == '┐').expect("corner");
    let bottom = (y..lines.len()).find(|&row| lines[row].get(x) == Some(&'└')).expect("bottom");
    assert!(x.abs_diff(80 - right - 1) <= 1, "{screen}");
    assert!((y - 1).abs_diff(23 - bottom - 1) <= 1, "{screen}");
    assert!(screen.contains("▸ updated just now · a"), "{screen}");
}

#[test]
fn error_screen_hint_click_quits_with_a_rebound_quit_key() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("config.toml");
    std::fs::write(&file, "[keys]\nquit = \"Z\"\n").expect("config");
    let config = Config::load(&file).expect("config");
    let mut app = App::new(state(Agent::Pi, "idle"), Screen::Error("no reply".into()), Vec::new(), &config);
    let screen = render(&mut app, 80, 12);
    let row = screen.lines().last().expect("footer");
    let byte = row.find("Z quit").expect("the quit hint is in the footer");
    let column = u16::try_from(row[..byte].width()).expect("column");
    let at = (column, 11);
    let down = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: at.0,
        row: at.1,
        modifiers: KeyModifiers::NONE,
    };
    assert_eq!(app.handle_mouse(down), Effect::Quit);
}

#[test]
fn snapshot_update_notice_80x24() {
    let mut app = viewer();
    let (sender, receiver) = std::sync::mpsc::channel();
    sender.send("Annorex 9.9.9 is available. Run: annorex update".to_owned()).expect("send");
    app.start_update_check(receiver);
    assert!(app.poll_background());
    let screen = render(&mut app, 80, 24);
    assert!(screen.contains("Annorex 9.9.9 is available. Run: annorex update"), "{screen}");
    insta::assert_snapshot!(screen);
}
