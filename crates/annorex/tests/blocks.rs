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
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

fn state() -> AgentState {
    AgentState { agent: Some(Agent::ClaudeCode), safety: Safety::Clear { state: Some("idle".to_owned()) } }
}

fn reply(id: &str, text: &str) -> Reply {
    Reply { id: id.to_owned(), text: text.to_owned(), at: Some("2026-11-09T10:30:30.000Z".to_owned()) }
}

fn open(text: &str) -> App {
    let mut app = open_raw(text);
    // The view starts without a focus: `j` focuses the first block.
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    app
}

/// The viewer as it starts: no block has the focus.
fn open_raw(text: &str) -> App {
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
fn enter_keeps_block_mode_and_the_strip_covers_the_block() {
    let mut app = open(DOC);
    press(&mut app, "j\n");
    assert_eq!(app.mode(), Mode::Block(1), "enter keeps block mode");
    assert_eq!(app.focused_block(), Some(1));
    assert!(strip_at(&render(&mut app, 120, 30)).is_some(), "enter opens the strip");
    // Block 1 has two source lines: a note from the strip covers the whole block.
    press(&mut app, "c");
    assert_eq!(editor_range(&app), app.doc().blocks[1].range);
}

#[test]
fn enter_without_a_focus_focuses_the_first_block_and_opens_the_strip() {
    let mut app = open_raw(DOC);
    press(&mut app, "\n");
    assert_eq!(app.mode(), Mode::Block(0));
    assert!(strip_at(&render(&mut app, 120, 30)).is_some());
}

#[test]
fn the_footer_and_the_help_name_enter() {
    let mut app = open(DOC);
    let footer = render(&mut app, 160, 30).lines().last().expect("footer").to_owned();
    assert!(footer.contains("enter actions"), "{footer}");
    press(&mut app, "?");
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("enter  show the action bar"), "{screen}");
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
    press(&mut app, "jj0j\u{1b}");
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

/// The buffer after one draw.
fn buffer_of(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    terminal.backend().buffer().clone()
}

/// The rows of the screen where column 0 shows the focus bar.
fn bar_rows(app: &mut App, width: u16, height: u16) -> Vec<u16> {
    let buffer = buffer_of(app, width, height);
    (0..height).filter(|&y| buffer.cell((0, y)).is_some_and(|cell| cell.symbol() == "\u{258d}")).collect()
}

/// The rows of the screen where the text `needle` starts.
fn rows_with(app: &mut App, needle: &str, width: u16, height: u16) -> Vec<u16> {
    let screen = render(app, width, height);
    (0..height).zip(screen.lines()).filter(|(_, line)| line.contains(needle)).map(|(y, _)| y).collect()
}

#[test]
fn markdown_has_no_line_numbers() {
    let mut app = open(DOC);
    let screen = render(&mut app, 80, 24);
    for line in screen.lines() {
        let trimmed = line.trim_start_matches(['\u{258d}', ' ']);
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        assert!(digits == 0 || !trimmed[digits..].starts_with(' '), "{line:?}\n{screen}");
    }
    assert!(screen.contains("First paragraph here"), "{screen}");
    let buffer = buffer_of(&mut app, 80, 24);
    let row = rows_with(&mut app, "Title", 80, 24)[0];
    // Margin column 0, then the text at column 1.
    assert_eq!(buffer.cell((1, row)).map(|c| c.symbol().to_owned()).as_deref(), Some("T"));
}

#[test]
fn plain_files_keep_line_numbers_and_the_bar_is_at_column_zero() {
    let mut app = open("x");
    app.open_file(annorex::files::LoadedFile {
        relative: std::path::PathBuf::from("a.rs"),
        text: "fn a() {}\n\nfn b() {}\n".to_owned(),
        stamp: annorex::files::Stamp { size: 22, modified: None },
    });
    // A new document starts without a focus.
    press(&mut app, "j");
    let screen = render(&mut app, 80, 24);
    let line = screen.lines().find(|line| line.contains("fn b()")).expect("line");
    assert!(line.trim_start_matches(['\u{258d}', ' ']).starts_with("3 fn b()"), "{screen}");
    assert_eq!(bar_rows(&mut app, 80, 24), rows_with(&mut app, "fn a()", 80, 24), "{screen}");
}

#[test]
fn the_focused_block_has_a_bar() {
    let mut app = open(DOC);
    let first = rows_with(&mut app, "First paragraph", 80, 24)[0];
    assert_eq!(bar_rows(&mut app, 80, 24), vec![rows_with(&mut app, "Title", 80, 24)[0]]);
    press(&mut app, "j");
    assert_eq!(bar_rows(&mut app, 80, 24), vec![first]);
    let buffer = buffer_of(&mut app, 80, 24);
    let cell = buffer.cell((0, first)).expect("cell");
    assert_eq!(cell.bg, Color::DarkGray);
    assert_eq!(buffer.cell((10, first)).expect("cell").bg, Color::DarkGray);
    let other = rows_with(&mut app, "Second paragraph", 80, 24)[0];
    assert_ne!(buffer.cell((10, other)).expect("cell").bg, Color::DarkGray);
    // A block of several rows: the bar is on each of them.
    press(&mut app, "G");
    let alpha = rows_with(&mut app, "alpha", 80, 24)[0];
    let gamma = rows_with(&mut app, "gamma", 80, 24)[0];
    let bars = bar_rows(&mut app, 80, 24);
    assert!(gamma > alpha && bars.len() > usize::from(gamma - alpha), "{bars:?}");
    assert!((alpha..=gamma).all(|y| bars.contains(&y)), "{bars:?}");
    assert!(bars.windows(2).all(|pair| pair[1] == pair[0] + 1), "one block: {bars:?}");
}

#[test]
fn text_mode_has_no_bar_and_the_text_does_not_shift() {
    let mut app = open(DOC);
    let before = render(&mut app, 80, 24);
    press(&mut app, "0");
    assert_eq!(bar_rows(&mut app, 80, 24), Vec::<u16>::new());
    let after = render(&mut app, 80, 24);
    let col = |screen: &str| {
        screen.lines().find(|l| l.contains("Title")).and_then(|l| l.chars().position(|c| c == 'T'))
    };
    assert_eq!(col(&before), col(&after));
}

#[test]
fn block_mode_draws_no_cursor_cell_but_text_mode_does() {
    let mut app = open(DOC);
    let reversed = |app: &mut App| {
        let buffer = buffer_of(app, 80, 24);
        let row = rows_with(app, "First paragraph", 80, 24)[0];
        (0..80)
            .filter(|&x| buffer.cell((x, row)).is_some_and(|c| c.modifier.contains(Modifier::REVERSED)))
            .count()
    };
    press(&mut app, "j");
    assert_eq!(reversed(&mut app), 0);
    press(&mut app, "0");
    assert_eq!(reversed(&mut app), 1);
}

#[test]
fn ctrl_d_at_the_end_keeps_the_focus_and_ctrl_u_at_the_start_too() {
    let mut app = open(DOC);
    render(&mut app, 80, 24);
    let last = app.doc().blocks.len() - 1;
    press(&mut app, "G");
    render(&mut app, 80, 24);
    press_key(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
    render(&mut app, 80, 24);
    assert_eq!(app.mode(), Mode::Block(last));
    press_key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    assert_eq!(app.mode(), Mode::Block(last));
    press(&mut app, "g");
    render(&mut app, 80, 24);
    press_key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert_eq!(app.mode(), Mode::Block(0));
    press_key(&mut app, KeyCode::PageUp, KeyModifiers::NONE);
    assert_eq!(app.mode(), Mode::Block(0));
}

#[test]
fn a_forward_scroll_never_focuses_a_lower_block() {
    let mut text = String::new();
    for i in 0..40 {
        writeln!(text, "paragraph {i:02}\n").expect("write");
    }
    let mut app = open(&text);
    render(&mut app, 80, 12);
    press(&mut app, "G");
    render(&mut app, 80, 12);
    press_key(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
    assert_eq!(app.mode(), Mode::Block(39));
    press(&mut app, "g");
    render(&mut app, 80, 12);
    press_key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert_eq!(app.mode(), Mode::Block(0));
}

#[test]
fn the_focus_background_fills_every_row_of_the_block_to_the_last_column() {
    let text = "intro\n\n```\nalpha\n\nbeta\n```\n\nafter\n";
    let mut app = open(text);
    press(&mut app, "j");
    let top = rows_with(&mut app, "alpha", 80, 24)[0];
    let buffer = buffer_of(&mut app, 80, 24);
    let bg = |x: u16, y: u16| buffer.cell((x, y)).expect("cell").bg;
    // The fence has a blank row between alpha and beta. Every row has the bar and the background
    // in the first column, in a column past the text, and in the last column of the doc area.
    let beta = rows_with(&mut app, "beta", 80, 24)[0];
    assert!(beta - top >= 2, "a blank row lies inside the block");
    for y in top..=beta {
        for x in [0, 30, 79] {
            assert_eq!(bg(x, y), Color::DarkGray, "cell ({x}, {y})");
        }
    }
    // The rows outside the block have no background.
    let after = rows_with(&mut app, "after", 80, 24)[0];
    assert_ne!(bg(79, after), Color::DarkGray);
    assert_ne!(bg(79, rows_with(&mut app, "intro", 80, 24)[0]), Color::DarkGray);
    // Text mode has no focus background.
    press(&mut app, "0");
    let buffer = buffer_of(&mut app, 80, 24);
    assert_ne!(buffer.cell((79, top)).expect("cell").bg, Color::DarkGray);
}

#[test]
fn a_note_key_on_an_empty_range_in_text_mode_says_nothing_to_annotate() {
    let mut app = open("alpha\n");
    press(&mut app, "0");
    assert_eq!(app.mode(), Mode::Text);
    // The same reply with no text keeps text mode, so the cursor line is gone.
    app.set_replies(vec![reply("r1", "")]);
    assert_eq!(app.mode(), Mode::Text);
    for key in ["c", "d", "s", "o"] {
        press(&mut app, key);
        assert!(app.editor().is_none(), "{key}");
        assert_eq!(app.status(), Some("nothing to annotate"), "{key}");
    }
}

#[test]
fn a_shown_empty_document_says_block_0_of_0() {
    let mut app = open("");
    assert!(app.shown_reply_id().is_some(), "an empty document is on the screen");
    let screen = render(&mut app, 80, 24);
    let footer = screen.lines().last().unwrap_or_default();
    assert!(footer.starts_with("block 0/0"), "{footer}");
    press(&mut app, "jkGg");
    assert!(render(&mut app, 80, 24).lines().last().unwrap_or_default().starts_with("block 0/0"));
}

// ---- No focus at the start (block mode without a focused block) ----

const CHOOSE: &str = "choose a block first (j/k or a click)";

/// A document of `count` paragraphs `para 0`, `para 1`, and so on: every block has two rows.
fn paragraphs(count: usize) -> String {
    (0..count).map(|n| format!("para {n}\n\n")).collect::<Vec<_>>().concat()
}

/// The numbers `N` of the `para N` words that the screen shows, in order.
fn shown_paragraphs(screen: &str) -> Vec<usize> {
    screen
        .lines()
        .filter_map(|line| line.split("para ").nth(1)?.split_whitespace().next()?.parse().ok())
        .collect()
}

fn cells(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    terminal.backend().buffer().clone()
}

#[test]
fn nothing_is_focused_at_the_start() {
    let mut app = open_raw(DOC);
    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(app.focused_block(), None);
    let screen = render(&mut app, 100, 30);
    assert!(!screen.contains('\u{258d}'), "no focus bar:\n{screen}");
    assert!(!screen.contains("looks good"), "no action bar:\n{screen}");
}

#[test]
fn a_new_document_starts_without_a_focus() {
    let mut app = open_raw(DOC);
    press(&mut app, "jjj");
    assert_eq!(app.focused_block(), Some(2));
    app.set_replies(vec![reply("r2", "other\n\ntext\n"), reply("r1", DOC)]);
    assert_eq!(app.mode(), Mode::Browse, "another reply starts without a focus");
}

#[test]
fn j_focuses_the_first_block_on_the_screen_and_k_the_last() {
    let mut app = open_raw(DOC);
    press(&mut app, "j");
    assert_eq!(app.mode(), Mode::Block(0));
    let mut app = open_raw(DOC);
    press(&mut app, "k");
    assert_eq!(app.mode(), Mode::Block(5), "k focuses the last block of a document that fits");

    let mut app = open_raw(&paragraphs(30));
    render(&mut app, 80, 12);
    press_key(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
    let screen = render(&mut app, 80, 12);
    assert_eq!(app.mode(), Mode::Browse, "ctrl+d only scrolls");
    let shown = shown_paragraphs(&screen);
    assert!(shown[0] > 0, "the view scrolled:\n{screen}");
    press(&mut app, "j");
    assert_eq!(app.focused_block(), shown.first().copied(), "{screen}");

    let mut app = open_raw(&paragraphs(30));
    render(&mut app, 80, 12);
    press_key(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
    let screen = render(&mut app, 80, 12);
    press(&mut app, "k");
    assert_eq!(app.focused_block(), shown_paragraphs(&screen).last().copied(), "{screen}");
}

#[test]
fn g_and_shift_g_scroll_without_a_focus() {
    let mut app = open_raw(&paragraphs(30));
    render(&mut app, 80, 12);
    press(&mut app, "G");
    let screen = render(&mut app, 80, 12);
    assert_eq!(app.mode(), Mode::Browse);
    assert!(screen.contains("para 29"), "{screen}");
    press(&mut app, "g");
    let screen = render(&mut app, 80, 12);
    assert_eq!(app.mode(), Mode::Browse);
    assert!(screen.contains("para 0"), "{screen}");
}

#[test]
fn esc_removes_the_focus_and_a_second_esc_changes_nothing() {
    let mut app = open_raw(DOC);
    press(&mut app, "jjj");
    press(&mut app, "\u{1b}");
    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(app.focused_block(), None);
    let screen = render(&mut app, 100, 30);
    assert!(!screen.contains('\u{258d}'), "{screen}");
    press(&mut app, "\u{1b}");
    assert_eq!(app.mode(), Mode::Browse);
    press(&mut app, "j");
    assert_eq!(app.mode(), Mode::Block(0), "j chooses a block again");
}

#[test]
fn a_note_key_without_a_focus_asks_for_a_block() {
    for key in "codsex".chars() {
        let mut app = open_raw(DOC);
        press(&mut app, &key.to_string());
        assert_eq!(app.status(), Some(CHOOSE), "key {key}");
        assert!(app.editor().is_none(), "key {key}");
        assert!(app.review().is_none_or(|review| review.annotations.is_empty()), "key {key}");
        assert_eq!(app.mode(), Mode::Browse, "key {key}");
    }
    let mut app = open_raw("");
    press(&mut app, "c");
    assert_eq!(app.status(), Some("nothing to annotate"), "an empty document has no block to choose");
}

#[test]
fn the_text_keys_start_at_the_first_block_on_the_screen() {
    let mut app = open_raw(DOC);
    press(&mut app, "l");
    assert_eq!(app.mode(), Mode::Text);
    assert_eq!(app.cursor(), app.doc().source.find("Title").expect("title") + 1);
}

#[test]
fn a_reload_of_the_same_document_keeps_the_focus_or_the_lack_of_it() {
    let changed = DOC.replace("Second paragraph.", "Second paragraph, new.");
    let mut app = open_raw(DOC);
    app.set_replies(vec![reply("r1", &changed)]);
    assert_eq!(app.mode(), Mode::Browse, "no focus stays no focus");
    let mut app = open_raw(DOC);
    press(&mut app, "jjj");
    app.set_replies(vec![reply("r1", &changed)]);
    assert_eq!(app.mode(), Mode::Block(2), "a focus stays");
}

#[test]
fn the_footer_without_a_focus_names_the_choice() {
    let mut app = open_raw(DOC);
    let screen = render(&mut app, 120, 30);
    let footer = screen.lines().last().expect("footer").to_owned();
    let choose = footer.find("j/k choose a block").unwrap_or_else(|| panic!("{footer}"));
    let send = footer.find("send").unwrap_or_else(|| panic!("{footer}"));
    let help = footer.find("? help").unwrap_or_else(|| panic!("{footer}"));
    assert!(choose < send && send < help, "{footer}");
    assert!(footer.contains("block -/6") || footer.contains("no block"), "{footer}");
    press(&mut app, "j");
    let footer = render(&mut app, 120, 30).lines().last().expect("footer").to_owned();
    assert!(footer.contains("block 1/6") && !footer.contains("choose a block"), "{footer}");
}

/// The action strip with the default keys.
const STRIP: &str =
    "\u{2590} o \u{2713} looks good  c \u{1f4ac} comment  d \u{2715} remove  s \u{21c4} replace \u{258c}";

/// The row and the screen column of the action strip, or `None` when no strip shows.
fn strip_at(screen: &str) -> Option<(usize, usize)> {
    screen.lines().enumerate().find_map(|(row, line)| {
        let index = line.find('\u{2590}').filter(|&index| line[index..].starts_with(STRIP))?;
        Some((row, line[..index].width()))
    })
}

/// The screen column of the first character of `needle` on its row.
fn column_of(screen: &str, needle: &str) -> usize {
    let line =
        screen.lines().find(|line| line.contains(needle)).unwrap_or_else(|| panic!("{needle}\n{screen}"));
    line[..line.find(needle).expect("needle")].width()
}

#[test]
fn j_focus_shows_no_strip() {
    let mut app = open(DOC);
    press(&mut app, "j");
    assert_eq!(app.mode(), Mode::Block(1));
    let screen = render(&mut app, 120, 30);
    assert_eq!(strip_at(&screen), None, "{screen}");
    assert!(!screen.contains("looks good"), "{screen}");
    // The note keys still act on the focus without the strip.
    press(&mut app, "c");
    assert_eq!(editor_range(&app), app.doc().blocks[1].range);
}

#[test]
fn enter_on_block_shows_strip() {
    let mut app = open(DOC);
    press(&mut app, "j\n");
    let screen = render(&mut app, 120, 30);
    let first = rows_with(&mut app, "First paragraph", 120, 30)[0];
    let (row, column) = strip_at(&screen).unwrap_or_else(|| panic!("no strip:\n{screen}"));
    assert_eq!(row, usize::from(first) - 1, "the row above the block:\n{screen}");
    assert_eq!(column, column_of(&screen, "First paragraph"), "the first column of the block:\n{screen}");
    // A move closes the strip.
    press(&mut app, "l");
    assert_eq!(strip_at(&render(&mut app, 120, 30)), None);
}

#[test]
fn strip_goes_below_at_top() {
    let mut app = open(DOC);
    // Block 0 is on the first document row: no row above it.
    press(&mut app, "\n");
    let screen = render(&mut app, 120, 30);
    let title = rows_with(&mut app, "Title", 120, 30)[0];
    let (row, column) = strip_at(&screen).unwrap_or_else(|| panic!("no strip:\n{screen}"));
    assert_eq!(row, usize::from(title) + 1, "the row below the block:\n{screen}");
    assert_eq!(column, column_of(&screen, "Title"));
}

#[test]
fn v_end_shows_strip() {
    let mut app = open(DOC);
    press(&mut app, "jvlll");
    assert_eq!(app.mode(), Mode::Text);
    let screen = render(&mut app, 120, 30);
    assert_eq!(strip_at(&screen), None, "no strip while the selection grows:\n{screen}");
    press(&mut app, "v");
    let screen = render(&mut app, 120, 30);
    let first = rows_with(&mut app, "First paragraph", 120, 30)[0];
    let (row, _) = strip_at(&screen).unwrap_or_else(|| panic!("no strip:\n{screen}"));
    assert_eq!(row, usize::from(first) - 1, "{screen}");
    // The second `v` keeps the selection: a note covers it.
    let start = app.doc().blocks[1].range.start;
    press(&mut app, "c");
    assert_eq!(editor_range(&app), start..start + 4);
}

#[test]
fn esc_closes_the_strip() {
    let mut app = open(DOC);
    press(&mut app, "j\n");
    assert!(strip_at(&render(&mut app, 120, 30)).is_some());
    press(&mut app, "\u{1b}");
    assert_eq!(strip_at(&render(&mut app, 120, 30)), None);
}

#[test]
fn the_strip_items_use_the_card_icons_and_colors() {
    let mut app = open(DOC);
    press(&mut app, "j\n");
    let buffer = cells(&mut app, 120, 30);
    let screen = render(&mut app, 120, 30);
    let (row, column) = strip_at(&screen).unwrap_or_else(|| panic!("no strip:\n{screen}"));
    let y = u16::try_from(row).expect("row");
    let x = u16::try_from(column).expect("column");
    let style = |x: u16| buffer.cell((x, y)).expect("cell").style();
    let strip = Color::Indexed(238);
    // The caps: the strip color on the default background.
    assert_eq!((style(x).fg, style(x).bg), (Some(strip), Some(Color::Reset)), "left cap");
    let width = u16::try_from(STRIP.width()).expect("width");
    let right = x + width - 1;
    assert_eq!((style(right).fg, style(right).bg), (Some(strip), Some(Color::Reset)), "right cap");
    // Each item: the key, the icon, and the word, bold, in the note color, on the strip color.
    for (word, color) in [
        ("o \u{2713} looks good", Color::Green),
        ("c \u{1f4ac} comment", Color::Yellow),
        ("d \u{2715} remove", Color::Red),
        ("s \u{21c4} replace", Color::Magenta),
    ] {
        let line = screen.lines().nth(row).expect("row");
        let at = u16::try_from(line[..line.find(word).expect("item")].width()).expect("column");
        // The first cell of each character. The cell after a wide character is reset: the terminal
        // draws the character over it with the style of the first cell.
        let starts = word.chars().scan(at, |x, c| {
            let start = *x;
            *x += u16::try_from(c.to_string().width()).expect("width");
            Some(start)
        });
        for cell in starts {
            let style = style(cell);
            assert_eq!(style.bg, Some(strip), "{word} cell {cell}");
            assert_eq!(style.fg, Some(color), "{word} cell {cell}");
            assert!(style.add_modifier.contains(Modifier::BOLD), "{word} cell {cell}");
        }
    }
    // The space between two items and the space inside the caps have the strip color.
    assert_eq!(style(x + 1).bg, Some(strip));
    assert_eq!(style(right - 1).bg, Some(strip));
    // The rest of the row keeps its own cells.
    assert_ne!(style(right + 1).bg, Some(strip));
}

#[test]
fn the_strip_leaves_out_the_items_that_do_not_fit() {
    let mut app = open(DOC);
    press(&mut app, "j\n");
    let screen = render(&mut app, 50, 20);
    let row = screen
        .lines()
        .find(|line| line.contains('\u{2590}'))
        .unwrap_or_else(|| panic!("no strip:\n{screen}"));
    assert!(row.contains("d \u{2715} remove \u{258c}"), "the last item that fits ends the strip: {row:?}");
    assert!(!row.contains("replace"), "{row:?}");
}
