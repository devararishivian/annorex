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

/// The label prefix of the reply viewer. The target agent block follows it.
const LAST_PREFIX: &str = "annorex:";
/// The label prefix of the file viewer. The target agent block follows it.
const FILES_PREFIX: &str = "annorex-files:";

/// The bounds of the layer: the full window.
const FULL_WINDOW: [u8; 4] = [0, 0, 1, 1];

/// The label of the viewer pane for one agent pane.
pub fn pane_label(target: &str) -> String {
    format!("{LAST_PREFIX}{target}")
}

/// The label of the files viewer pane for one agent pane.
pub fn pane_label_files(target: &str) -> String {
    format!("{FILES_PREFIX}{target}")
}

/// Which viewer to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View<'a> {
    /// The replies of the agent.
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

/// The viewer and the agent block of an Annorex label, or `None` for another label.
fn parse_label(label: &str) -> Option<(&'static str, &str)> {
    if let Some(target) = label.strip_prefix(FILES_PREFIX) {
        Some(("files", target))
    } else {
        label.strip_prefix(LAST_PREFIX).map(|target| ("last", target))
    }
}

/// A label of Annorex has a block id after the prefix, so a label such as `annorex: notes` does not count.
fn is_annorex(block: &BlockInfo) -> bool {
    parse_label(&block.label).is_some_and(|(_, target)| target.starts_with("block:"))
}

/// The `session.new_layer` request for a viewer of `view`: the full `window`, with focus. File paths
/// follow `--`, so a name that starts with `-` is not read as an option.
pub fn layer_request_for(
    view: &View<'_>,
    target: &str,
    session: &str,
    window: &str,
    exe: &Path,
    cwd: Option<&Path>,
) -> serde_json::Value {
    let mut command: Vec<String> =
        [exe.to_string_lossy().as_ref(), view.name(), "--target", target, "--session", session]
            .map(str::to_owned)
            .into();
    if let View::Files(paths) = view
        && !paths.is_empty()
    {
        command.push("--".to_owned());
        command.extend(paths.iter().cloned());
    }
    let label = match view {
        View::Last => pane_label(target),
        View::Files(_) => pane_label_files(target),
    };
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
    /// `last` or `files`.
    pub viewer: String,
    /// The agent block that the open Annorex serves.
    pub target: String,
    /// The name of that agent (`claude-code`, `pi`, or `opencode`), when Rex still reports it.
    pub agent: Option<String>,
    /// True when the open Annorex serves the agent block that the user asked for.
    pub same_target: bool,
    /// True when the open Annorex is the viewer that the user asked for.
    pub same_viewer: bool,
}

impl Existing {
    /// What to tell the user when the open Annorex is not the one that was asked for: the agent it
    /// serves, the viewer it shows, and how to close it. `None` when it is the one asked for.
    pub fn notice(&self) -> Option<String> {
        if self.same_target && self.same_viewer {
            return None;
        }
        let viewer = if self.viewer == "files" { "file viewer" } else { "reply viewer" };
        let agent = self.agent.as_deref().unwrap_or("an agent");
        Some(format!(
            "Annorex is already open in this session, for {agent} ({viewer}). It is in front now; press q there to close it, then start Annorex again."
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
    open_view(api, &View::Last, target, session, exe)
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
    open_view(api, &View::Files(paths), target, session, exe)
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
fn describe(api: &impl OpenApi, found: &Located<'_>, view: &View<'_>, target: &str) -> Option<Existing> {
    let (viewer, served) = parse_label(&found.block.label)?;
    let agent = api
        .process(served)
        .ok()
        .zip(api.program_status(served).ok())
        .and_then(|(process, status)| detect(&process, &status))
        .map(|agent: Agent| agent.status_app().to_owned());
    Some(Existing {
        viewer: viewer.to_owned(),
        target: served.to_owned(),
        agent,
        same_target: served == target,
        same_viewer: viewer == view.name(),
    })
}

/// Show the open Annorex `found` and say so.
fn focus_existing(
    api: &impl OpenApi,
    found: &Located<'_>,
    view: &View<'_>,
    target: &str,
) -> Result<Opened, OpenError> {
    show(api, found)?;
    Ok(Opened {
        block_id: found.block.block_id.clone(),
        focused_existing: true,
        existing: describe(api, found, view, target),
    })
}

/// Open the viewer of `view` for the agent in `target` over the full window of the agent. A session has one
/// Annorex: if one is open, show it and open nothing.
pub fn open_view(
    api: &impl OpenApi,
    view: &View<'_>,
    target: &str,
    session: &str,
    exe: &Path,
) -> Result<Opened, OpenError> {
    let process = api.process(target)?;
    let status = api.program_status(target)?;
    if detect(&process, &status).is_none() {
        return Err(OpenError::NotAgent);
    }
    let before = api.view()?;
    if let Some(found) = before.locate(is_annorex).first() {
        return focus_existing(api, found, view, target);
    }
    let window = api.window_of(target)?;
    let cwd = process.foreground.as_ref().and_then(|fg| fg.cwd.as_deref());
    let request = layer_request_for(view, target, session, &window, exe, cwd);
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
            return focus_existing(api, oldest, view, target);
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
        assert_eq!(block["label"], format!("annorex:{TARGET}"));
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
        assert_eq!(error.to_string(), "the focused pane is not running Claude Code, Pi, or OpenCode");
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
        assert!(existing.same_target && existing.same_viewer);
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
        assert!(!existing.same_target && existing.same_viewer);
        assert!(!fake.called().contains(&"new_layer".to_owned()));
    }

    // S5: the file viewer is open and the user starts the reply viewer, and the reverse.
    #[test]
    fn the_other_viewer_counts_as_the_one_annorex() {
        let files_open = view(&[("layer:a", None, &[("block:a", &pane_label_files(TARGET))])]);
        let opened = open(&Fake::claude(vec![files_open])).expect("open");
        let existing = opened.existing.expect("existing");
        assert_eq!(existing.viewer, "files");
        assert!(existing.same_target && !existing.same_viewer);
        let last_open = view(&[("layer:a", None, &[("block:a", &pane_label(TARGET))])]);
        let fake = Fake::claude(vec![last_open]);
        let opened = open_files(&fake, TARGET, "session:s", &exe(), &[]).expect("open");
        let existing = opened.existing.expect("existing");
        assert_eq!(existing.viewer, "last");
        assert!(!existing.same_viewer);
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
        );
        let block = &request["layout"]["block"];
        assert_eq!(block["label"], format!("annorex-files:{TARGET}"));
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
    fn labels_fit_the_rex_limit_and_differ() {
        assert_eq!(pane_label(TARGET).len(), 50);
        assert!(pane_label_files(TARGET).len() <= 128);
        assert_ne!(pane_label_files(TARGET), pane_label(TARGET));
        assert_eq!(parse_label(&pane_label_files(TARGET)), Some(("files", TARGET)));
        assert_eq!(parse_label(&pane_label(TARGET)), Some(("last", TARGET)));
        assert_eq!(parse_label("claude"), None);
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
    fn the_notice_names_the_agent_and_the_viewer_when_they_differ() {
        let same = Existing {
            viewer: "last".into(),
            target: TARGET.into(),
            agent: Some("pi".into()),
            same_target: true,
            same_viewer: true,
        };
        assert_eq!(same.notice(), None);
        let other = Existing { same_target: false, ..same.clone() };
        let text = other.notice().expect("notice");
        assert!(text.contains("pi") && text.contains("reply") && text.contains("press q"), "{text}");
        let files = Existing { viewer: "files".into(), same_viewer: false, ..same };
        assert!(files.notice().expect("notice").contains("file"));
    }

    #[test]
    fn a_label_that_only_starts_with_the_prefix_is_not_an_annorex() {
        let user =
            BlockInfo { block_id: "block:u".into(), label: "annorex: notes".into(), creator_name: None };
        assert!(!is_annorex(&user));
        let ours =
            BlockInfo { block_id: "block:o".into(), label: format!("annorex:{TARGET}"), creator_name: None };
        assert!(is_annorex(&ours));
    }
}
