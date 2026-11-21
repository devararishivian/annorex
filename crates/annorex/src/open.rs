//! `annorex rex open last` and `annorex rex open files`: open the viewer in a floating layer over the
//! full window of the agent, or show the viewer that is already open in the session.

use std::path::Path;

use crate::agent::{Agent, detect};
use crate::rex::{
    BlockInfo, LayerKind, LayerState, Located, NewLayer, ProcessInfo, Rex, RexError, Runner, SessionView,
    StatusList,
};

/// The flavor of a new terminal block.
pub const TERMINAL_FLAVOR: &str = "com.superlogical.terminal.shell";

/// The label prefix of the viewer. The target agent block follows it, so the id stays last.
const LABEL_PREFIX: &str = "Annorex review · ";
/// The label prefix of version 0.2.0. An open layer with it still counts as the one Annorex.
const V0_2_PREFIX: &str = "annorex:";
/// The label prefix of the file viewer of M3 to M5. An open layer with it still counts as the one Annorex.
const OLD_FILES_PREFIX: &str = "annorex-files:";

/// The bounds of the layer: the full window.
const FULL_WINDOW: [u8; 4] = [0, 0, 1, 1];

/// The label of the viewer pane for one agent pane.
pub fn pane_label(target: &str) -> String {
    format!("{LABEL_PREFIX}{target}")
}

/// What the viewer shows first. Both start the same viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View<'a> {
    /// The newest reply of the agent.
    Last,
    /// The files of the work directory. The paths open at the start.
    Files(&'a [String]),
}

impl View<'_> {
    fn name(&self) -> &'static str {
        match self {
            View::Last => "last",
            View::Files(_) => "files",
        }
    }
}

/// The agent block of an Annorex label, or `None` for another label.
fn parse_label(label: &str) -> Option<&str> {
    [LABEL_PREFIX, V0_2_PREFIX, OLD_FILES_PREFIX].iter().find_map(|prefix| label.strip_prefix(prefix))
}

/// A label of Annorex has a block id after the prefix, so a label such as `annorex: notes` does not count.
pub(crate) fn is_annorex(block: &BlockInfo) -> bool {
    parse_label(&block.label).is_some_and(|target| target.starts_with("block:"))
}

/// The `session.new_layer` request for a viewer of `view`: the full `window`, with focus. File paths
/// follow `--`, so a name that starts with `-` is not read as an option. With `no_update_check`, the
/// viewer does not check for a newer release.
pub fn layer_request_for(
    view: &View<'_>,
    target: &str,
    session: &str,
    window: &str,
    exe: &Path,
    cwd: Option<&Path>,
    no_update_check: bool,
) -> serde_json::Value {
    let mut command: Vec<String> =
        [exe.to_string_lossy().as_ref(), view.name(), "--target", target, "--session", session]
            .map(str::to_owned)
            .into();
    if no_update_check {
        command.push("--no-update-check".to_owned());
    }
    if let View::Files(paths) = view
        && !paths.is_empty()
    {
        command.push("--".to_owned());
        command.extend(paths.iter().cloned());
    }
    let label = pane_label(target);
    let mut options = serde_json::Map::new();
    options.insert("command".to_owned(), serde_json::json!(command));
    if let Some(cwd) = cwd {
        options.insert("cwd".to_owned(), serde_json::Value::from(cwd.to_string_lossy().as_ref()));
    }
    serde_json::json!({
        "window_id": window,
        "bounds": FULL_WINDOW,
        "focus": true,
        "layout": {
            "block": { "flavor": TERMINAL_FLAVOR, "label": label, "options": options },
        },
    })
}

/// The Rex calls that `open_view` needs. `Rex` implements it. Tests use a fake.
pub trait OpenApi {
    fn process(&self, block: &str) -> Result<ProcessInfo, RexError>;
    fn program_status(&self, block: &str) -> Result<StatusList, RexError>;
    fn view(&self) -> Result<SessionView, RexError>;
    fn window_of(&self, block: &str) -> Result<String, RexError>;
    fn new_layer(&self, request: &serde_json::Value) -> Result<NewLayer, RexError>;
    fn set_layer_state(&self, layer: &str, state: LayerState, focus: bool) -> Result<(), RexError>;
    fn close_layer(&self, layer: &str) -> Result<(), RexError>;
    fn focus_block(&self, block: &str) -> Result<(), RexError>;
}

impl<R: Runner> OpenApi for Rex<R> {
    fn process(&self, block: &str) -> Result<ProcessInfo, RexError> {
        Rex::process(self, block)
    }
    fn program_status(&self, block: &str) -> Result<StatusList, RexError> {
        Rex::program_status(self, block)
    }
    fn view(&self) -> Result<SessionView, RexError> {
        Rex::view(self)
    }
    fn window_of(&self, block: &str) -> Result<String, RexError> {
        Rex::window_of(self, block)
    }
    fn new_layer(&self, request: &serde_json::Value) -> Result<NewLayer, RexError> {
        Rex::new_layer(self, request)
    }
    fn set_layer_state(&self, layer: &str, state: LayerState, focus: bool) -> Result<(), RexError> {
        Rex::set_layer_state(self, layer, state, focus)
    }
    fn close_layer(&self, layer: &str) -> Result<(), RexError> {
        Rex::close_layer(self, layer)
    }
    fn focus_block(&self, block: &str) -> Result<(), RexError> {
        Rex::focus_block(self, block)
    }
}

/// The Annorex that was already open in the session, when `open_view` showed it and opened nothing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Existing {
    /// The agent block that the open Annorex serves.
    pub target: String,
    /// The name of that agent (`claude-code`, `pi`, or `opencode`), when Rex still reports it.
    pub agent: Option<String>,
    /// True when the open Annorex serves the agent block that the user asked for.
    pub same_target: bool,
}

impl Existing {
    /// What to tell the user when the open Annorex serves another agent: which agent, and how to close it.
    /// `None` when it serves the agent that the user asked for.
    pub fn notice(&self) -> Option<String> {
        if self.same_target {
            return None;
        }
        let agent = self.agent.as_deref().unwrap_or("an agent");
        Some(format!(
            "Annorex is already open in this session, for {agent}. It is in front now; press q there to close it, then start Annorex again."
        ))
    }
}

/// The viewer pane after `open_view`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Opened {
    pub block_id: String,
    pub focused_existing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing: Option<Existing>,
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("{}", crate::agent::NOT_AGENT)]
    NotAgent,
    #[error("Rex did not give the id of the new pane")]
    NoNewBlock,
    #[error(transparent)]
    Rex(#[from] RexError),
}

/// Open the viewer for the agent in `target`. If the reply viewer of `target` is open, focus it.
pub fn open_last(api: &impl OpenApi, target: &str, session: &str, exe: &Path) -> Result<Opened, OpenError> {
    open_view(api, &View::Last, target, session, exe, false)
}

/// Open the files viewer for the agent in `target`, with `paths` open. If an Annorex is open in the
/// session, focus it. Then the paths are not used.
pub fn open_files(
    api: &impl OpenApi,
    target: &str,
    session: &str,
    exe: &Path,
    paths: &[String],
) -> Result<Opened, OpenError> {
    open_view(api, &View::Files(paths), target, session, exe, false)
}

/// Show an Annorex block that is already open: restore and raise its layer, or focus a pane of the old
/// split layout.
fn show(api: &impl OpenApi, found: &Located<'_>) -> Result<(), RexError> {
    if found.layer.kind == LayerKind::Floating {
        api.set_layer_state(&found.layer.layer_id, LayerState::Normal, true)
    } else {
        api.focus_block(&found.block.block_id)
    }
}

/// What the user needs to know about an open Annorex that is not the one that they asked for.
fn describe(api: &impl OpenApi, found: &Located<'_>, target: &str) -> Option<Existing> {
    let served = parse_label(&found.block.label)?;
    let agent = api
        .process(served)
        .ok()
        .zip(api.program_status(served).ok())
        .and_then(|(process, status)| detect(&process, &status))
        .map(|agent: Agent| agent.status_app().to_owned());
    Some(Existing { target: served.to_owned(), agent, same_target: served == target })
}

/// Show the open Annorex `found` and say so.
fn focus_existing(api: &impl OpenApi, found: &Located<'_>, target: &str) -> Result<Opened, OpenError> {
    show(api, found)?;
    Ok(Opened {
        block_id: found.block.block_id.clone(),
        focused_existing: true,
        existing: describe(api, found, target),
    })
}

/// Open the viewer of `view` for the agent in `target` over the full window of the agent. A session has one
/// Annorex: if one is open, show it and open nothing. With `no_update_check`, the viewer does not check
/// for a newer release.
pub fn open_view(
    api: &impl OpenApi,
    view: &View<'_>,
    target: &str,
    session: &str,
    exe: &Path,
    no_update_check: bool,
) -> Result<Opened, OpenError> {
    let process = api.process(target)?;
    let status = api.program_status(target)?;
    if detect(&process, &status).is_none() {
        return Err(OpenError::NotAgent);
    }
    let before = api.view()?;
    if let Some(found) = before.locate(is_annorex).first() {
        return focus_existing(api, found, target);
    }
    let window = api.window_of(target)?;
    let cwd = process.foreground.as_ref().and_then(|fg| fg.cwd.as_deref());
    let request = layer_request_for(view, target, session, &window, exe, cwd, no_update_check);
    let layer = api.new_layer(&request)?;
    let Some(block_id) = layer.block_ids.first().cloned() else {
        // Do not leave a layer without a known block.
        let _ = api.close_layer(&layer.layer_id);
        return Err(OpenError::NoNewBlock);
    };
    // Two starts at the same time can both find no Annorex. The oldest block stays. The newer one closes
    // its own layer. Block ids are time-ordered, so the smaller id is the older block. Both starts compare
    // the same ids in the same way, so exactly one stays, also when two ids have the same millisecond.
    if let Ok(after) = api.view() {
        let found = after.locate(is_annorex);
        if let Some(oldest) = found.iter().min_by(|a, b| a.block.block_id.cmp(&b.block.block_id))
            && oldest.block.block_id != block_id
        {
            // The kept Annorex is the one that matters. A failed close must not hide it.
            let _ = api.close_layer(&layer.layer_id);
            return focus_existing(api, oldest, target);
        }
    }
    Ok(Opened { block_id, focused_existing: false, existing: None })
}

/// Hand the focus back when the viewer quits: minimize the layer of `own_block` without focus, then
/// focus `target`. Without this, Rex puts the focus on a block of its own, which can be another
/// agent [live, 2026-11-11]. Every error is ignored, because the viewer is ending.
pub fn release_focus(api: &impl OpenApi, own_block: &str, target: &str) {
    let Ok(view) = api.view() else { return };
    let Some(layer) =
        view.locate(|block| block.block_id == own_block).first().map(|found| found.layer.layer_id.clone())
    else {
        return;
    };
    let _ = api.set_layer_state(&layer, LayerState::Minimized, false);
    let _ = api.focus_block(target);
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::*;

    const TARGET: &str = "block:01a1b000-0000-7000-8000-000000000004";
    const OTHER: &str = "block:01a1b000-0000-7000-8000-000000000008";
    const WINDOW: &str = "window:01a1b000-0000-7000-8000-000000000002";

    /// A floating layer for the test view: its id, its state, and its blocks `(block id, label)`.
    type Floating<'a> = (&'a str, Option<&'a str>, &'a [(&'a str, &'a str)]);

    /// A view with the agent block in the tiled layer and the given floating layers.
    fn view(floating: &[Floating<'_>]) -> Value {
        let mut layers = vec![json!({
            "layer_id": "layer:tiled", "kind": "tiled",
            "blocks": [{ "block_id": TARGET, "label": "claude" }],
        })];
        for (id, state, blocks) in floating {
            let blocks: Vec<Value> =
                blocks.iter().map(|(block, label)| json!({ "block_id": block, "label": label })).collect();
            let mut layer = json!({ "layer_id": id, "kind": "floating", "blocks": blocks });
            if let Some(state) = state {
                layer["state"] = json!(state);
            }
            layers.push(layer);
        }
        json!({ "windows": [{ "window_id": WINDOW, "focused_block_id": TARGET, "layers": layers }] })
    }

    struct Fake {
        process: &'static str,
        status: &'static str,
        /// The views that `view` gives, one for each call. The last one repeats.
        views: Vec<Value>,
        calls: RefCell<Vec<String>>,
        requests: RefCell<Vec<Value>>,
        new_layer_fails: bool,
        window: Option<&'static str>,
    }

    impl Fake {
        fn claude(views: Vec<Value>) -> Self {
            Self {
                process: include_str!("../tests/fixtures/rex/process-claude.json"),
                status: include_str!("../tests/fixtures/rex/status-claude-working.json"),
                views,
                calls: RefCell::default(),
                requests: RefCell::default(),
                new_layer_fails: false,
                window: Some(WINDOW),
            }
        }
        fn called(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
        fn note(&self, text: impl Into<String>) {
            self.calls.borrow_mut().push(text.into());
        }
    }

    impl OpenApi for Fake {
        fn process(&self, block: &str) -> Result<ProcessInfo, RexError> {
            self.note(format!("process {}", &block[block.len() - 4..]));
            Ok(serde_json::from_str(self.process).expect("process"))
        }
        fn program_status(&self, block: &str) -> Result<StatusList, RexError> {
            self.note(format!("program_status {}", &block[block.len() - 4..]));
            Ok(serde_json::from_str(self.status).expect("status"))
        }
        fn view(&self) -> Result<SessionView, RexError> {
            let index = self.calls.borrow().iter().filter(|call| *call == "view").count();
            self.note("view");
            let value = self.views.get(index).or(self.views.last()).expect("a view");
            Ok(serde_json::from_value(value.clone()).expect("view"))
        }
        fn window_of(&self, block: &str) -> Result<String, RexError> {
            self.note(format!("window_of {}", &block[block.len() - 4..]));
            self.window.map(str::to_owned).ok_or(RexError::NotInWindow { block: block.to_owned() })
        }
        fn new_layer(&self, request: &Value) -> Result<NewLayer, RexError> {
            self.note("new_layer");
            self.requests.borrow_mut().push(request.clone());
            if self.new_layer_fails {
                return Err(RexError::Failed {
                    method: "session.new_layer".into(),
                    code: Some(1),
                    stderr: "bad".into(),
                });
            }
            Ok(NewLayer { layer_id: "layer:new".into(), block_ids: vec!["block:new".into()] })
        }
        fn set_layer_state(&self, layer: &str, state: LayerState, focus: bool) -> Result<(), RexError> {
            self.note(format!("set_layer_state {layer} {state:?} {focus}"));
            Ok(())
        }
        fn close_layer(&self, layer: &str) -> Result<(), RexError> {
            self.note(format!("close_layer {layer}"));
            Ok(())
        }
        fn focus_block(&self, block: &str) -> Result<(), RexError> {
            self.note(format!("focus_block {}", &block[block.len() - 4..]));
            Ok(())
        }
    }

    fn exe() -> PathBuf {
        PathBuf::from("/usr/local/bin/annorex")
    }

    fn open(fake: &Fake) -> Result<Opened, OpenError> {
        open_last(fake, TARGET, "session:s", &exe())
    }

    fn agent_only() -> Value {
        view(&[])
    }

    // S1: one agent, one full-window layer in the window of the agent.
    #[test]
    fn the_viewer_opens_as_a_full_window_layer_with_focus_in_the_window_of_the_agent() {
        let fake = Fake::claude(vec![
            agent_only(),
            view(&[("layer:new", None, &[("block:new", &pane_label(TARGET))])]),
        ]);
        let opened = open(&fake).expect("open");
        assert_eq!(opened, Opened { block_id: "block:new".into(), focused_existing: false, existing: None });
        let request = fake.requests.borrow()[0].clone();
        assert_eq!(request["window_id"], WINDOW);
        assert_eq!(request["bounds"], json!([0, 0, 1, 1]));
        assert_eq!(request["focus"], true);
        let block = &request["layout"]["block"];
        assert_eq!(block["flavor"], TERMINAL_FLAVOR);
        assert_eq!(block["label"], format!("Annorex review · {TARGET}"));
        assert_eq!(
            block["options"]["command"],
            json!(["/usr/local/bin/annorex", "last", "--target", TARGET, "--session", "session:s"])
        );
        assert_eq!(block["options"]["cwd"], "/Users/dev/work/project");
        assert_eq!(request.get("anchor_id"), None, "no split");
    }

    // S11: no layer opens for a pane without an agent.
    #[test]
    fn a_pane_without_an_agent_opens_nothing() {
        let fake = Fake {
            process: include_str!("../tests/fixtures/rex/process-shell.json"),
            status: include_str!("../tests/fixtures/rex/status-empty.json"),
            ..Fake::claude(vec![agent_only()])
        };
        let error = open(&fake).expect_err("not an agent");
        assert!(matches!(error, OpenError::NotAgent));
        assert_eq!(error.to_string(), "the focused pane is not running Claude Code, Pi, OpenCode, or Codex");
        assert_eq!(fake.called(), ["process 0004", "program_status 0004"]);
    }

    // S3: the same viewer again shows the open layer and opens no second one.
    #[test]
    fn the_open_viewer_is_shown_not_duplicated() {
        let open_view_json = view(&[("layer:a", None, &[("block:a", &pane_label(TARGET))])]);
        let fake = Fake::claude(vec![open_view_json]);
        let opened = open(&fake).expect("open");
        assert!(opened.focused_existing);
        assert_eq!(opened.block_id, "block:a");
        let existing = opened.existing.expect("existing");
        assert!(existing.same_target);
        assert!(!fake.called().contains(&"new_layer".to_owned()));
        assert!(
            fake.called().contains(&"set_layer_state layer:a Normal true".to_owned()),
            "{:?}",
            fake.called()
        );
    }

    // S4: the viewer for agent A is open. The user starts it on agent B in the same session.
    #[test]
    fn a_viewer_for_another_agent_is_shown_and_named() {
        let fake = Fake::claude(vec![view(&[("layer:a", None, &[("block:a", &pane_label(OTHER))])])]);
        let opened = open(&fake).expect("open");
        assert!(opened.focused_existing);
        let existing = opened.existing.expect("existing");
        assert_eq!(existing.target, OTHER);
        assert_eq!(existing.agent.as_deref(), Some("claude-code"));
        assert!(!existing.same_target);
        assert!(!fake.called().contains(&"new_layer".to_owned()));
    }

    // S5: `last` and `files` use one label, so either command shows the open Annorex.
    #[test]
    fn last_and_files_use_the_same_label() {
        let request =
            |view: &View<'_>| layer_request_for(view, TARGET, "session:s", WINDOW, &exe(), None, false);
        assert_eq!(request(&View::Last)["layout"]["block"]["label"], pane_label(TARGET));
        assert_eq!(request(&View::Files(&[]))["layout"]["block"]["label"], pane_label(TARGET));
    }

    #[test]
    fn the_open_viewer_is_shown_for_either_command() {
        let open_json = view(&[("layer:a", None, &[("block:a", &pane_label(TARGET))])]);
        let fake = Fake::claude(vec![open_json.clone()]);
        let opened = open_files(&fake, TARGET, "session:s", &exe(), &[]).expect("open");
        assert!(opened.focused_existing && opened.existing.expect("existing").same_target);
        assert!(!fake.called().contains(&"new_layer".to_owned()));
        let fake = Fake::claude(vec![open_json]);
        assert!(open(&fake).expect("open").focused_existing);
        assert!(!fake.called().contains(&"new_layer".to_owned()));
    }

    // An Annorex of v0.2.0 has the label `annorex:block:<id>`. It counts after the upgrade.
    #[test]
    fn a_layer_with_the_v0_2_label_counts_as_the_one_annorex() {
        let old = view(&[("layer:a", None, &[("block:a", &format!("annorex:{TARGET}"))])]);
        let fake = Fake::claude(vec![old]);
        let opened = open(&fake).expect("open");
        assert!(opened.focused_existing);
        assert!(!fake.called().contains(&"new_layer".to_owned()));
    }

    // An Annorex of M5 has the label `annorex-files:`. It counts after the upgrade.
    #[test]
    fn a_layer_with_the_old_files_label_counts_as_the_one_annorex() {
        let old = view(&[("layer:a", None, &[("block:a", &format!("annorex-files:{TARGET}"))])]);
        let fake = Fake::claude(vec![old]);
        let opened = open(&fake).expect("open");
        assert!(opened.focused_existing);
        assert!(!fake.called().contains(&"new_layer".to_owned()));
    }

    // An old and a new layer in one session: the oldest viewer stays and the other one closes.
    #[test]
    fn an_old_and_a_new_label_in_one_session_keep_the_oldest_viewer() {
        let (old_id, new_id) =
            ("block:01a1b000-0000-7000-8000-0000000000a1", "block:01a1b000-0000-7000-8000-0000000000b2");
        let both = view(&[
            ("layer:old", None, &[(old_id, &format!("annorex:{TARGET}"))]),
            ("layer:new", None, &[(new_id, &pane_label(TARGET))]),
        ]);
        let fake = Fake::claude(vec![both]);
        let opened = open(&fake).expect("open");
        assert!(opened.focused_existing);
        assert_eq!(opened.block_id, old_id);
        assert!(!fake.called().contains(&"new_layer".to_owned()));
    }

    // S7: a minimized layer comes back, raised and focused.
    #[test]
    fn a_minimized_layer_is_restored_with_focus() {
        let fake =
            Fake::claude(vec![view(&[("layer:a", Some("minimized"), &[("block:a", &pane_label(TARGET))])])]);
        open(&fake).expect("open");
        assert!(fake.called().contains(&"set_layer_state layer:a Normal true".to_owned()));
    }

    // S14: a pane of the old split layout counts as the one Annorex and gets the focus.
    #[test]
    fn a_viewer_pane_of_the_old_split_layout_is_focused() {
        let old = json!({ "windows": [{ "window_id": WINDOW, "layers": [{
            "layer_id": "layer:tiled", "kind": "tiled",
            "blocks": [{ "block_id": TARGET, "label": "claude" },
                       { "block_id": "block:old", "label": pane_label(TARGET) }],
        }] }] });
        let fake = Fake::claude(vec![old]);
        let opened = open(&fake).expect("open");
        assert!(opened.focused_existing);
        assert!(fake.called().contains(&"focus_block :old".to_owned()), "{:?}", fake.called());
        assert!(!fake.called().contains(&"new_layer".to_owned()));
    }

    // S8: two starts at the same time. The newer layer closes itself and the older one is shown.
    #[test]
    fn the_newer_of_two_simultaneous_viewers_closes_its_own_layer() {
        let (old_id, new_id) = ("block:01a1b000-0000-7000-8000-0000000000a1", "block:new");
        let both = view(&[
            ("layer:old", None, &[(old_id, &pane_label(TARGET))]),
            ("layer:new", None, &[(new_id, &pane_label(TARGET))]),
        ]);
        let fake = Fake::claude(vec![agent_only(), both]);
        let opened = open(&fake).expect("open");
        assert!(opened.focused_existing);
        assert_eq!(opened.block_id, old_id);
        let calls = fake.called();
        assert!(calls.contains(&"close_layer layer:new".to_owned()), "{calls:?}");
        assert!(calls.contains(&"set_layer_state layer:old Normal true".to_owned()), "{calls:?}");
    }

    #[test]
    fn the_older_of_two_simultaneous_viewers_stays_open() {
        // The new block of the fake is `block:new`. It sorts before `block:zzz-later`, so it is the older one.
        let both = view(&[
            ("layer:new", None, &[("block:new", &pane_label(TARGET))]),
            ("layer:other", None, &[("block:zzz-later", &pane_label(TARGET))]),
        ]);
        let fake = Fake::claude(vec![agent_only(), both]);
        let opened = open(&fake).expect("open");
        assert!(!opened.focused_existing);
        assert_eq!(opened.block_id, "block:new");
        assert!(!fake.called().iter().any(|call| call.starts_with("close_layer")), "{:?}", fake.called());
    }

    // S9: the layer opens in the window of the agent block, also when that window is not active.
    #[test]
    fn the_window_comes_from_the_agent_block() {
        let fake = Fake::claude(vec![agent_only()]);
        open(&fake).expect("open");
        assert!(fake.called().contains(&"window_of 0004".to_owned()));
    }

    // S16: a failed call gives the error and leaves nothing to clean up.
    #[test]
    fn a_failed_new_layer_is_an_error_and_closes_nothing() {
        let fake = Fake { new_layer_fails: true, ..Fake::claude(vec![agent_only()]) };
        let error = open(&fake).expect_err("fails");
        assert!(matches!(error, OpenError::Rex(RexError::Failed { .. })), "{error:?}");
        assert!(!fake.called().iter().any(|call| call.starts_with("close_layer")));
    }

    #[test]
    fn an_agent_block_in_no_window_is_an_error() {
        let fake = Fake { window: None, ..Fake::claude(vec![agent_only()]) };
        let error = open(&fake).expect_err("no window");
        assert!(matches!(error, OpenError::Rex(RexError::NotInWindow { .. })), "{error:?}");
        assert!(!fake.called().contains(&"new_layer".to_owned()));
    }

    #[test]
    fn the_files_request_passes_the_paths_after_a_double_dash() {
        let request = layer_request_for(
            &View::Files(&["README.md".to_owned(), "-odd name.md".to_owned()]),
            TARGET,
            "session:s",
            WINDOW,
            &exe(),
            None,
            false,
        );
        let block = &request["layout"]["block"];
        assert_eq!(block["label"], pane_label(TARGET));
        assert_eq!(
            block["options"]["command"],
            json!([
                "/usr/local/bin/annorex",
                "files",
                "--target",
                TARGET,
                "--session",
                "session:s",
                "--",
                "README.md",
                "-odd name.md"
            ])
        );
        assert_eq!(block["options"].get("cwd"), None);
    }

    #[test]
    fn the_request_has_the_update_flag_only_when_asked() {
        let command = |view: &View<'_>, no_update_check: bool| {
            let request = layer_request_for(view, TARGET, "session:s", WINDOW, &exe(), None, no_update_check);
            request["layout"]["block"]["options"]["command"].clone()
        };
        let base = ["/usr/local/bin/annorex", "last", "--target", TARGET, "--session", "session:s"];
        assert_eq!(command(&View::Last, false), json!(base));
        assert_eq!(
            command(&View::Last, true),
            json!([
                "/usr/local/bin/annorex",
                "last",
                "--target",
                TARGET,
                "--session",
                "session:s",
                "--no-update-check"
            ])
        );
        let paths = ["a.md".to_owned()];
        assert_eq!(
            command(&View::Files(&paths), true),
            json!([
                "/usr/local/bin/annorex",
                "files",
                "--target",
                TARGET,
                "--session",
                "session:s",
                "--no-update-check",
                "--",
                "a.md"
            ])
        );
    }

    #[test]
    fn the_label_fits_the_rex_limit_and_the_old_files_label_is_still_read() {
        assert_eq!(pane_label(TARGET), format!("Annorex review · {TARGET}"));
        assert!(pane_label(TARGET).chars().count() <= 128);
        assert!(pane_label(TARGET).ends_with(TARGET), "the id stays last");
        assert_eq!(parse_label(&pane_label(TARGET)), Some(TARGET));
        assert_eq!(parse_label(&format!("annorex:{TARGET}")), Some(TARGET));
        assert_eq!(parse_label(&format!("annorex-files:{TARGET}")), Some(TARGET));
        assert_eq!(parse_label("claude"), None);
        assert_eq!(parse_label("Annorex review · notes"), Some("notes"));
    }

    // S2: on quit, the viewer hands the focus to the agent. Without this, Rex picks a block of its own.
    #[test]
    fn quit_minimizes_the_own_layer_without_focus_and_focuses_the_agent() {
        let fake = Fake::claude(vec![view(&[("layer:mine", None, &[("block:mine", &pane_label(TARGET))])])]);
        release_focus(&fake, "block:mine", TARGET);
        assert_eq!(
            fake.called(),
            vec![
                "view".to_owned(),
                "set_layer_state layer:mine Minimized false".to_owned(),
                format!("focus_block {}", &TARGET[TARGET.len() - 4..]),
            ]
        );
    }

    #[test]
    fn quit_does_nothing_when_the_own_block_is_in_no_layer() {
        let fake = Fake::claude(vec![agent_only()]);
        release_focus(&fake, "block:mine", TARGET);
        assert_eq!(fake.called(), vec!["view".to_owned()]);
    }

    #[test]
    fn the_notice_names_the_agent_when_the_target_differs() {
        let same = Existing { target: TARGET.into(), agent: Some("pi".into()), same_target: true };
        assert_eq!(same.notice(), None);
        let other = Existing { same_target: false, ..same };
        let text = other.notice().expect("notice");
        assert!(text.contains("pi") && text.contains("press q"), "{text}");
    }

    #[test]
    fn a_label_that_only_starts_with_the_prefix_is_not_an_annorex() {
        for label in ["annorex: notes", "Annorex review · notes", "Annorex review", "my block:x"] {
            let user = BlockInfo { block_id: "block:u".into(), label: label.into(), creator_name: None };
            assert!(!is_annorex(&user), "{label}");
        }
        for label in [pane_label(TARGET), format!("annorex:{TARGET}"), format!("annorex-files:{TARGET}")] {
            let ours = BlockInfo { block_id: "block:o".into(), label, creator_name: None };
            assert!(is_annorex(&ours));
        }
    }
}
