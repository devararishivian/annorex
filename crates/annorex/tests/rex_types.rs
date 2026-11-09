//! The Rex JSON types parse live output of `rex block call`.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use annorex::rex::{Mode, Modes, ProcessInfo, SessionList, SessionView, Size, StatusList};
use serde::de::DeserializeOwned;

fn fixture<T: DeserializeOwned>(name: &str) -> T {
    let path = format!("{}/tests/fixtures/rex/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).expect("fixture");
    serde_json::from_str(&text).expect("fixture parses")
}

#[test]
fn every_process_fixture_parses() {
    for name in ["process-claude", "process-pi", "process-opencode", "process-shell"] {
        let process: ProcessInfo = fixture(name);
        assert!(process.foreground.is_some(), "{name}");
    }
}

#[test]
fn pi_runs_as_node_with_argv0_pi() {
    let process: ProcessInfo = fixture("process-pi");
    let fg = process.foreground.expect("foreground");
    assert_eq!(fg.argv0.as_deref(), Some("pi"));
    assert_eq!(fg.name, "node");
}

#[test]
fn every_status_fixture_parses() {
    for name in [
        "status-claude-working",
        "status-claude-blocked",
        "status-pi-idle",
        "status-opencode-idle",
        "status-empty",
    ] {
        let _: StatusList = fixture(name);
    }
}

#[test]
fn claude_working_has_a_root_record() {
    let status: StatusList = fixture("status-claude-working");
    let root = status.root().expect("root record");
    assert_eq!(root.app.as_deref(), Some("claude-code"));
    assert_eq!(root.state, "working");
}

#[test]
fn claude_blocked_has_kind_question() {
    let status: StatusList = fixture("status-claude-blocked");
    let root = status.root().expect("root record");
    assert_eq!(root.state, "blocked");
    assert_eq!(root.kind.as_deref(), Some("question"));
    assert_eq!(root.msg.as_deref(), Some("answer: Pick a color. (Red · Blue)"));
}

#[test]
fn empty_status_has_no_root() {
    let status: StatusList = fixture("status-empty");
    assert!(status.root().is_none());
}

#[test]
fn agent_modes_have_bracketed_paste() {
    let modes: Modes = fixture("modes-agent");
    assert!(modes.bracketed_paste());
}

#[test]
fn zsh_at_its_prompt_has_bracketed_paste() {
    let modes: Modes = fixture("modes-shell");
    assert!(modes.bracketed_paste());
}

#[test]
fn a_program_without_bracketed_paste_reports_it_off() {
    let modes: Modes = fixture("modes-no-paste");
    assert!(!modes.bracketed_paste());
}

#[test]
fn ansi_mode_with_number_2004_is_not_bracketed_paste() {
    let modes = Modes {
        screen: "primary".to_owned(),
        modes: Some(vec![Mode {
            name: "bracketed_paste".to_owned(),
            number: 2004,
            ansi: Some(true),
            set: true,
        }]),
    };
    assert!(!modes.bracketed_paste());
}

#[test]
fn modes_without_a_list_have_no_bracketed_paste() {
    let modes = Modes { screen: "primary".to_owned(), modes: None };
    assert!(!modes.bracketed_paste());
}

#[test]
fn size_wide_gives_218_columns() {
    let size: Size = fixture("size-wide");
    assert_eq!((size.columns, size.rows), (218, 69));
    let narrow: Size = fixture("size-narrow");
    assert_eq!((narrow.columns, narrow.rows), (120, 40));
}

#[test]
fn size_larger_than_u16_saturates() {
    let size: Size = serde_json::from_str(r#"{"columns":70000,"rows":5}"#).expect("parses");
    assert_eq!((size.columns, size.rows), (u16::MAX, 5));
}

#[test]
fn view_with_annorex_finds_the_label() {
    let view: SessionView = fixture("view-with-annorex");
    let found = view.find_label("annorex:block:01a1b000-0000-7000-8000-000000000004").expect("annorex pane");
    assert_eq!(found.block_id, "block:01a1b000-0000-7000-8000-000000000099");
    assert_eq!(found.creator_name.as_deref(), Some("com.superlogical.terminal"));
    assert_eq!(view.blocks().count(), 2);
}

#[test]
fn view_agent_only_has_no_annorex_label() {
    let view: SessionView = fixture("view-agent-only");
    assert!(view.blocks().all(|block| !block.label.starts_with("annorex:")));
    assert_eq!(view.blocks().count(), 1);
}

#[test]
fn session_list_gives_ids_in_order() {
    let list: SessionList = fixture("session-list");
    assert_eq!(
        list.ids(),
        [
            "session:01a1b000-0000-7000-8000-000000000013",
            "session:01a1b000-0000-7000-8000-000000000014",
            "session:01a1b000-0000-7000-8000-000000000001",
        ]
    );
}
