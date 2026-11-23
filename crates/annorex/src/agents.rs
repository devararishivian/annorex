//! `annorex rex agents`: the agent panes of a session, for the question of the Lua module when the
//! focused pane is not an agent.

use std::path::Path;

use crate::agent::{Agent, detect};
use crate::open::{OpenApi, is_annorex};
use crate::replies::TERMINAL_CREATOR;
use crate::rex::RexError;

/// One agent pane of the session.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AgentPane {
    /// The agent block, for example `block:...`.
    pub block_id: String,
    pub agent: Agent,
    /// The name of the agent, for example `Codex`.
    pub title: String,
    /// The work directory of the agent, with the home folder as `~`.
    pub subtitle: String,
    /// The label of the window, or `tab N` when the window has no label, and the position of the pane in
    /// the window, for example `work · pane 2`.
    pub detail: String,
}

/// The terminal blocks of the session that run an agent, in the order of `session.view`: windows,
/// then layers, then blocks. An Annorex viewer is not in the list. A block whose `process` or
/// `program_status` call fails is not in the list either.
pub fn agent_panes(api: &impl OpenApi, home: &Path) -> Result<Vec<AgentPane>, RexError> {
    let view = api.view()?;
    let mut panes = Vec::new();
    for (index, window) in view.windows.iter().enumerate() {
        let detail =
            if window.label.trim().is_empty() { format!("tab {}", index + 1) } else { window.label.clone() };
        // The position counts every block of the window, in the order of the view.
        let blocks = window.layers.iter().flat_map(|layer| layer.blocks.iter()).enumerate();
        for (position, block) in blocks.filter(|(_, block)| {
            block.creator_name.as_deref() == Some(TERMINAL_CREATOR) && !is_annorex(block)
        }) {
            let Ok(process) = api.process(&block.block_id) else { continue };
            let Ok(status) = api.program_status(&block.block_id) else { continue };
            let Some(agent) = detect(&process, &status) else { continue };
            let cwd = process.foreground.as_ref().and_then(|fg| fg.cwd.as_deref());
            panes.push(AgentPane {
                block_id: block.block_id.clone(),
                agent,
                title: agent.host().display_name().to_owned(),
                subtitle: cwd.map(|cwd| short_path(cwd, home)).unwrap_or_default(),
                detail: format!("{detail} · pane {}", position + 1),
            });
        }
    }
    Ok(panes)
}

/// `path` with the home folder as `~`. Only a whole folder name counts, and only an absolute `home`.
fn short_path(path: &Path, home: &Path) -> String {
    if home.is_absolute()
        && let Ok(rest) = path.strip_prefix(home)
    {
        return if rest.as_os_str().is_empty() { "~".to_owned() } else { format!("~/{}", rest.display()) };
    }
    path.display().to_string()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::*;
    use crate::open::pane_label;
    use crate::rex::{LayerState, NewLayer, ProcessInfo, SessionView, StatusList};

    const CLAUDE: &str = "block:01a1b000-0000-7000-8000-0000000000c1";
    const CODEX: &str = "block:01a1b000-0000-7000-8000-0000000000c2";
    const SHELL: &str = "block:01a1b000-0000-7000-8000-0000000000c3";
    const VIEWER: &str = "block:01a1b000-0000-7000-8000-0000000000c4";
    const BROKEN: &str = "block:01a1b000-0000-7000-8000-0000000000c5";
    const NOTE: &str = "block:01a1b000-0000-7000-8000-0000000000c6";
    const NO_STATUS: &str = "block:01a1b000-0000-7000-8000-0000000000c7";
    const CODEX_TWO: &str = "block:01a1b000-0000-7000-8000-0000000000c8";

    /// A fake Rex: each block gives the process and the status of one fixture.
    struct Fake {
        view: Value,
        /// The blocks of the `program_status` calls.
        status_calls: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            Self { view: session(), status_calls: RefCell::default() }
        }
    }

    fn fixture(block: &str) -> (&'static str, &'static str) {
        match block {
            CLAUDE => (
                include_str!("../tests/fixtures/rex/process-claude.json"),
                include_str!("../tests/fixtures/rex/status-claude-working.json"),
            ),
            CODEX | CODEX_TWO | VIEWER | NO_STATUS => (
                include_str!("../tests/fixtures/rex/process-codex.json"),
                include_str!("../tests/fixtures/rex/status-codex-idle.json"),
            ),
            _ => (
                include_str!("../tests/fixtures/rex/process-shell.json"),
                include_str!("../tests/fixtures/rex/status-empty.json"),
            ),
        }
    }

    fn failed(method: &str) -> RexError {
        RexError::Failed { method: method.into(), code: Some(1), stderr: "gone".into() }
    }

    impl OpenApi for Fake {
        fn process(&self, block: &str) -> Result<ProcessInfo, RexError> {
            if block == BROKEN {
                return Err(failed("process"));
            }
            Ok(serde_json::from_str(fixture(block).0).expect("process"))
        }
        fn program_status(&self, block: &str) -> Result<StatusList, RexError> {
            self.status_calls.borrow_mut().push(block.to_owned());
            if block == NO_STATUS {
                return Err(failed("program_status"));
            }
            Ok(serde_json::from_str(fixture(block).1).expect("status"))
        }
        fn view(&self) -> Result<SessionView, RexError> {
            Ok(serde_json::from_value(self.view.clone()).expect("view"))
        }
        fn window_of(&self, _block: &str) -> Result<String, RexError> {
            panic!("not used")
        }
        fn new_layer(&self, _request: &Value) -> Result<NewLayer, RexError> {
            panic!("not used")
        }
        fn set_layer_state(&self, _layer: &str, _state: LayerState, _focus: bool) -> Result<(), RexError> {
            panic!("not used")
        }
        fn close_layer(&self, _layer: &str) -> Result<(), RexError> {
            panic!("not used")
        }
        fn focus_block(&self, _block: &str) -> Result<(), RexError> {
            panic!("not used")
        }
    }

    fn block(id: &str, label: &str, creator: Option<&str>) -> Value {
        json!({ "block_id": id, "label": label, "creator_name": creator })
    }

    /// Window 1 has a label, the Claude Code pane, and a shell pane. Window 2 has no label, the Codex
    /// pane, a pane whose `process` fails, a Codex pane whose `program_status` fails, an Annorex viewer over it, and a block that is not a terminal.
    fn session() -> Value {
        json!({ "windows": [
            { "window_id": "window:1", "label": "work", "layers": [
                { "layer_id": "layer:t1", "kind": "tiled", "blocks": [
                    block(CLAUDE, "claude", Some(TERMINAL_CREATOR)),
                    block(SHELL, "zsh", Some(TERMINAL_CREATOR)),
                ] },
            ] },
            { "window_id": "window:2", "label": "", "layers": [
                { "layer_id": "layer:t2", "kind": "tiled", "blocks": [
                    block(CODEX, "codex", Some(TERMINAL_CREATOR)),
                    block(BROKEN, "x", Some(TERMINAL_CREATOR)),
                    block(NO_STATUS, "y", Some(TERMINAL_CREATOR)),
                    block(NOTE, "notes", Some("com.superlogical.notes")),
                ] },
                { "layer_id": "layer:f2", "kind": "floating", "blocks": [
                    block(VIEWER, &pane_label(CODEX), Some(TERMINAL_CREATOR)),
                ] },
            ] },
        ] })
    }

    fn home() -> PathBuf {
        PathBuf::from("/Users/dev")
    }

    #[test]
    fn the_agent_panes_come_in_view_order_with_their_names_folders_and_windows() {
        let panes = agent_panes(&Fake::new(), &home()).expect("panes");
        assert_eq!(
            panes,
            vec![
                AgentPane {
                    block_id: CLAUDE.into(),
                    agent: Agent::ClaudeCode,
                    title: "Claude Code".into(),
                    subtitle: "~/work/project".into(),
                    detail: "work · pane 1".into(),
                },
                AgentPane {
                    block_id: CODEX.into(),
                    agent: Agent::Codex,
                    title: "Codex CLI".into(),
                    subtitle: "~/work/project".into(),
                    detail: "tab 2 · pane 1".into(),
                },
            ]
        );
    }

    #[test]
    fn two_panes_of_one_agent_in_one_window_differ_by_their_position() {
        let mut fake = Fake::new();
        fake.view = json!({ "windows": [
            { "window_id": "window:1", "label": "Window 8c3a9dd3", "layers": [
                { "layer_id": "layer:t1", "kind": "tiled", "blocks": [
                    block(SHELL, "zsh", Some(TERMINAL_CREATOR)),
                    block(CODEX, "codex", Some(TERMINAL_CREATOR)),
                ] },
                { "layer_id": "layer:t2", "kind": "tiled", "blocks": [
                    block(CODEX_TWO, "codex", Some(TERMINAL_CREATOR)),
                ] },
            ] },
        ] });
        let panes = agent_panes(&fake, &home()).expect("panes");
        let details: Vec<&str> = panes.iter().map(|pane| pane.detail.as_str()).collect();
        assert_eq!(details, ["Window 8c3a9dd3 · pane 2", "Window 8c3a9dd3 · pane 3"]);
    }

    #[test]
    fn an_annorex_viewer_a_broken_pane_and_a_shell_are_not_listed() {
        let panes = agent_panes(&Fake::new(), &home()).expect("panes");
        let ids: Vec<&str> = panes.iter().map(|pane| pane.block_id.as_str()).collect();
        for left_out in [SHELL, VIEWER, BROKEN, NOTE, NO_STATUS] {
            assert!(!ids.contains(&left_out), "{left_out} in {ids:?}");
        }
    }

    #[test]
    fn a_pane_whose_process_call_fails_gets_no_status_call() {
        let fake = Fake::new();
        agent_panes(&fake, &home()).expect("panes");
        let calls = fake.status_calls.borrow();
        assert!(!calls.iter().any(|block| block == BROKEN), "{calls:?}");
        assert!(calls.iter().any(|block| block == NO_STATUS), "{calls:?}");
    }

    #[test]
    fn the_list_is_one_json_object_with_the_agent_names_of_the_status() {
        let panes = agent_panes(&Fake::new(), &home()).expect("panes");
        let text = serde_json::to_string(&json!({ "agents": panes })).expect("json");
        assert!(text.contains(r#""agent":"codex""#), "{text}");
        assert!(text.contains(r#""agent":"claude-code""#), "{text}");
    }

    #[test]
    fn the_home_folder_becomes_a_tilde_only_for_a_whole_folder_name() {
        assert_eq!(short_path(Path::new("/Users/dev"), &home()), "~");
        assert_eq!(short_path(Path::new("/Users/dev/a b"), &home()), "~/a b");
        assert_eq!(short_path(Path::new("/Users/devo/x"), &home()), "/Users/devo/x");
        assert_eq!(short_path(Path::new("/opt/x"), &home()), "/opt/x");
        assert_eq!(short_path(Path::new("/opt/x"), Path::new("")), "/opt/x");
    }
}
