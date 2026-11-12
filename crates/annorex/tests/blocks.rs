//! Block mode and text mode: the focus moves block by block, and text mode keeps the free cursor.

#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use std::fmt::Write as _;

use annorex::agent::Agent;
use annorex::config::Config;
use annorex::replies::Reply;
use annorex::safety::Safety;
use annorex::ui::{AgentState, App, Effect, Mode, Screen, draw};
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

fn open(text: &str) -> App {
    let mut app = App::new(state(), Screen::Viewer, vec![reply("r1", text)], &Config::default());
    app.set_time_zone(jiff::tz::TimeZone::UTC);
    app
}

fn press_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Effect {
    app.handle_key(KeyEvent::new(code, modifiers))
}

fn press(app: &mut App, keys: &str) {
    for c in keys.chars() {
        let code = match c {
            '\n' => KeyCode::Enter,
            '\u{1b}' => KeyCode::Esc,
            c => KeyCode::Char(c),
        };
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

/// Blocks: 0 heading, 1 paragraph of two source lines, 2 paragraph, 3 and 4 list items, 5 code fence.
const DOC: &str = "# Title\n\nFirst paragraph here\nstill the first.\n\nSecond paragraph.\n\n- one\n- two\n\n```\nalpha\nbeta\ngamma\n```\n";

fn block_text(app: &App, index: usize) -> String {
    let range = app.doc().blocks[index].range.clone();
    app.doc().source[range].to_owned()
}

/// Sixty lines with hard breaks: one paragraph block of sixty display lines. Then one more block.
fn tall_text() -> String {
    let mut text = String::new();
    for i in 0..60 {
        writeln!(text, "row {i:02}  ").expect("write");
    }
    text.push_str("\nafter\n");
    text
}

fn editor_range(app: &App) -> std::ops::Range<usize> {
    app.editor().expect("the editor is open").range.clone()
}

#[test]
fn starts_in_block_mode_on_the_first_block() {
    let app = open(DOC);
    assert_eq!(app.doc().blocks.len(), 6, "the fixture has six blocks");
    assert_eq!(app.mode(), Mode::Block(0));
    assert_eq!(app.focused_block(), Some(0));
    assert_eq!(app.cursor(), app.doc().source.find("Title").expect("title"));
}

#[test]
fn j_and_k_move_by_block() {
    let mut app = open(DOC);
    press(&mut app, "j");
    assert_eq!(app.mode(), Mode::Block(1));
    press(&mut app, "jj");
    assert_eq!(app.mode(), Mode::Block(3));
    press(&mut app, "k");
    assert_eq!(app.mode(), Mode::Block(2));
    press(&mut app, "jjjjjjjj");
    assert_eq!(app.mode(), Mode::Block(5), "j stops at the last block");
    press(&mut app, "kkkkkkkkk");
    assert_eq!(app.mode(), Mode::Block(0), "k stops at the first block");
    press_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.mode(), Mode::Block(1), "the arrow keys move too");
}

#[test]
fn g_and_shift_g_go_to_the_ends() {
    let mut app = open(DOC);
    press(&mut app, "G");
    assert_eq!(app.mode(), Mode::Block(5));
    press(&mut app, "g");
    assert_eq!(app.mode(), Mode::Block(0));
}

#[test]
fn a_tall_block_scrolls_before_j_moves_on() {
    let mut app = open(&tall_text());
    assert_eq!(app.doc().blocks.len(), 2);
    let first = render(&mut app, 80, 12);
    assert!(first.contains("row 00"), "{first}");
    let mut presses = 0;
    let mut previous = first;
    while app.mode() == Mode::Block(0) {
        assert!(presses < 60, "j never left the tall block");
        let saw_end = previous.contains("row 59");
        press(&mut app, "j");
        presses += 1;
        let screen = render(&mut app, 80, 12);
        if app.mode() == Mode::Block(0) {
            assert_ne!(screen, previous, "j scrolls inside the tall block");
            assert!(!saw_end, "j moves on when the end of the block is in view");
        } else {
            assert!(saw_end, "j moved on before the end of the block was in view:\n{previous}");
        }
        previous = screen;
    }
    assert_eq!(app.mode(), Mode::Block(1));
    assert!(presses > 2, "the tall block took {presses} presses");
    assert!(previous.contains("after"), "{previous}");

    // k works the same way up: it comes back to the end of the block, then scrolls to its start.
    press(&mut app, "k");
    let screen = render(&mut app, 80, 12);
    assert_eq!(app.mode(), Mode::Block(0));
    assert!(screen.contains("row 59"), "k shows the end of the tall block first:\n{screen}");
    press(&mut app, "k");
    assert_eq!(app.mode(), Mode::Block(0), "k scrolls inside the tall block");
    let screen = render(&mut app, 80, 12);
    assert!(!screen.contains("row 59"), "{screen}");
}

#[test]
fn a_tall_block_scrolls_without_a_draw_between_keys() {
    let mut app = open(&tall_text());
    render(&mut app, 80, 12);
    press(&mut app, "j");
    assert_eq!(app.mode(), Mode::Block(0));
    press(&mut app, "jjjjjjjjjjjjjjjjjjjj");
    assert_eq!(app.mode(), Mode::Block(1));
    let screen = render(&mut app, 80, 12);
    assert!(screen.contains("after"), "{screen}");
}

#[test]
fn ctrl_d_focuses_the_first_block_that_starts_in_the_view() {
    let mut text = String::new();
    for i in 0..40 {
        writeln!(text, "paragraph {i:02}\n").expect("write");
    }
    let mut app = open(&text);
    render(&mut app, 80, 12);
    press_key(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
    let screen = render(&mut app, 80, 12);
    let focused = app.focused_block().expect("a block has the focus");
    assert!(focused > 0, "ctrl+d moves the focus");
    assert!(matches!(app.mode(), Mode::Block(_)));
    let first_shown = screen.lines().find(|line| line.contains("paragraph ")).expect("a paragraph shows");
    assert!(first_shown.contains(&block_text(&app, focused)), "{screen}");
    press_key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    render(&mut app, 80, 12);
    assert_eq!(app.mode(), Mode::Block(0), "ctrl+u goes back");
}

#[test]
fn enter_goes_to_text_mode_at_the_block_start() {
    let mut app = open(DOC);
    press(&mut app, "j\n");
    assert_eq!(app.mode(), Mode::Text);
    assert_eq!(app.cursor(), app.doc().blocks[1].range.start);
    assert_eq!(app.focused_block(), Some(1), "text mode reports the block of the cursor");
    // No selection: a note after a move covers the line of the cursor, not a few characters.
    press(&mut app, "lllc");
    let line = &app.doc().lines[app.cursor_line()];
    assert_eq!(editor_range(&app), line.source);
}

#[test]
fn h_l_w_b_0_dollar_enter_text_mode_then_move() {
    let start = open(DOC).doc().blocks[1].range.start;
    let source = DOC;
    for (keys, expected) in [
        ("jl", start + 1),
        ("jh", start),
        ("jw", source.find("paragraph here").expect("word")),
        ("jb", source.find("Title").expect("title")),
        ("j$l0", start),
    ] {
        let mut app = open(DOC);
        press(&mut app, keys);
        assert_eq!(app.mode(), Mode::Text, "{keys}");
        assert_eq!(app.cursor(), expected, "{keys}");
    }
    let mut app = open(DOC);
    press(&mut app, "j$");
    assert_eq!(app.mode(), Mode::Text);
    let line = &app.doc().lines[app.cursor_line()];
    assert_eq!(app.cursor(), app.doc().stops(app.cursor_line()).last().copied().expect("stop"), "{line:?}");
}

#[test]
fn v_anchors_at_the_block_start() {
    let mut app = open(DOC);
    press(&mut app, "jv");
    assert_eq!(app.mode(), Mode::Text);
    press(&mut app, "lllllc");
    let start = app.doc().blocks[1].range.start;
    assert_eq!(editor_range(&app), start..start + 6);
    assert_eq!(&DOC[editor_range(&app)], "First ");
}

#[test]
fn shift_v_selects_the_block_lines() {
    let mut app = open(DOC);
    press(&mut app, "GV");
    assert_eq!(app.mode(), Mode::Text);
    press(&mut app, "c");
    let block = app.doc().blocks[5].clone();
    let lines = &app.doc().lines;
    let expected = lines[block.lines.start].source.start..lines[block.lines.end - 1].source.end;
    assert_eq!(editor_range(&app), expected);
    assert_eq!(&DOC[editor_range(&app)], "alpha\nbeta\ngamma");
}

#[test]
fn esc_clears_then_returns_to_block_mode() {
    let mut app = open(DOC);
    press(&mut app, "jvll\u{1b}");
    assert_eq!(app.mode(), Mode::Text, "the first esc only clears the selection");
    press(&mut app, "\u{1b}");
    assert_eq!(app.mode(), Mode::Block(1), "the second esc goes back to the block of the cursor");
    assert_eq!(app.cursor(), app.doc().blocks[1].range.start);

    // The first esc cleared the selection: a note covers the line of the cursor.
    let mut app = open(DOC);
    press(&mut app, "jvll\u{1b}c");
    let line = &app.doc().lines[app.cursor_line()];
    assert_eq!(editor_range(&app), line.source);

    // Without a selection, one esc goes back to block mode.
    let mut app = open(DOC);
    press(&mut app, "jj\nj\u{1b}");
    assert_eq!(app.mode(), Mode::Block(3), "j in text mode moves by line");
}

#[test]
fn c_in_block_mode_annotates_the_whole_block() {
    let mut app = open(DOC);
    press(&mut app, "jc");
    assert_eq!(editor_range(&app), app.doc().blocks[1].range);
    assert_eq!(&DOC[editor_range(&app)], "First paragraph here\nstill the first.");
    press(&mut app, "fine\n");
    assert_eq!(app.mode(), Mode::Block(1), "a saved note keeps block mode");

    let mut app = open(DOC);
    press(&mut app, "Go");
    let review = app.review().expect("review");
    assert_eq!(review.annotations.len(), 1);
    assert_eq!(review.annotations[0].range, app.doc().blocks[5].range, "o covers the whole fence");
}

#[test]
fn find_next_puts_the_cursor_on_the_match_in_text_mode() {
    let mut app = open(DOC);
    press(&mut app, "/para\n");
    assert_eq!(app.mode(), Mode::Text);
    assert_eq!(app.cursor(), DOC.find("paragraph here").expect("match"));
    press(&mut app, "n");
    assert_eq!(app.cursor(), DOC.find("paragraph.").expect("match"));
    press(&mut app, "\u{1b}\u{1b}N");
    assert_eq!(app.mode(), Mode::Text, "N from block mode goes to text mode");
    assert_eq!(app.cursor(), DOC.find("paragraph here").expect("match"));

    let mut app = open(DOC);
    press(&mut app, "/nothing like this\n");
    assert_eq!(app.mode(), Mode::Block(0), "no match keeps block mode");
    assert_eq!(app.status(), Some("no match"));
}

#[test]
fn an_empty_doc_has_no_focus_and_note_keys_say_nothing_to_annotate() {
    let mut app = open("");
    assert_eq!(app.focused_block(), None);
    for key in ["c", "d", "s", "o"] {
        press(&mut app, key);
        assert!(app.editor().is_none(), "{key}");
        assert_eq!(app.status(), Some("nothing to annotate"), "{key}");
    }
    for keys in ["j", "k", "g", "G", "\n", "v", "V", "l", "w", "$", "\u{1b}", "n"] {
        press(&mut app, keys);
        assert_eq!(app.focused_block(), None, "{keys:?}");
    }
    for code in [KeyCode::PageDown, KeyCode::PageUp, KeyCode::Home, KeyCode::End] {
        press_key(&mut app, code, KeyModifiers::NONE);
    }
    press_key(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
    press_key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    render(&mut app, 80, 24);
    render(&mut app, 40, 12);
    assert_eq!(app.focused_block(), None);
}

#[test]
fn a_new_read_keeps_the_focus_on_the_same_text() {
    let mut app = open("# Title\n\nalpha para\n\nbeta para\n");
    press(&mut app, "jj");
    assert_eq!(block_text(&app, 2), "beta para");
    assert_eq!(app.mode(), Mode::Block(2));
    app.set_replies(vec![reply("r1", "# Title\n\nnew intro para\n\nmore\n\nalpha para\n\nbeta para\n")]);
    let focused = app.focused_block().expect("focus");
    assert_eq!(block_text(&app, focused), "beta para", "the focus follows its text");
    assert_eq!(app.mode(), Mode::Block(4));
    assert_eq!(app.cursor(), app.doc().blocks[4].range.start);

    // The focused text is gone: the focus goes to the block near the old offset, without a panic.
    app.set_replies(vec![reply("r1", "# Title\n")]);
    assert_eq!(app.mode(), Mode::Block(0));
    app.set_replies(vec![reply("r1", "")]);
    assert_eq!(app.focused_block(), None);
    render(&mut app, 80, 24);
}
