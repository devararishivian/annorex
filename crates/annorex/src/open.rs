//! `annorex rex open last`: open the viewer in a split next to the agent pane, or focus the
//! viewer that is already open for that pane.

use std::path::Path;

use crate::agent::detect;
use crate::rex::{ProcessInfo, Rex, RexError, Runner, SessionView, Size, StatusList};

/// The flavor of a new terminal block.
pub const TERMINAL_FLAVOR: &str = "com.superlogical.terminal.shell";

/// At this width or more, the viewer goes next to the agent. Else it goes below.
pub const SIDE_BY_SIDE_COLUMNS: u16 = 160;

/// Where the viewer opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Placement {
    /// Next to the agent when the agent pane has 160 columns or more, else below.
    #[default]
    Auto,
    /// Always next to the agent.
    Right,
    /// Always below the agent.
    Below,
}

/// The label of the viewer pane for one agent pane.
pub fn pane_label(target: &str) -> String {
    format!("annorex:{target}")
}

/// The label of the files viewer pane for one agent pane.
pub fn pane_label_files(target: &str) -> String {
    format!("annorex-files:{target}")
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
    fn label(&self, target: &str) -> String {
        match self {
            View::Last => pane_label(target),
            View::Files(_) => pane_label_files(target),
        }
    }
}

/// The split direction: `horizontal` is side by side, `vertical` is one above the other.
pub fn direction(placement: Placement, columns: u16) -> &'static str {
    let side_by_side = match placement {
        Placement::Right => true,
        Placement::Below => false,
        Placement::Auto => columns >= SIDE_BY_SIDE_COLUMNS,
    };
    if side_by_side { "horizontal" } else { "vertical" }
}

/// The `session.new_split` request for the viewer of `target`.
pub fn split_request(
    target: &str,
    session: &str,
    exe: &Path,
    cwd: Option<&Path>,
    direction: &str,
) -> serde_json::Value {
    split_request_for(&View::Last, target, session, exe, cwd, direction)
}

/// The `session.new_split` request for a viewer of `view`. File paths follow `--`, so a name
/// that starts with `-` is not read as an option.
pub fn split_request_for(
    view: &View<'_>,
    target: &str,
    session: &str,
    exe: &Path,
    cwd: Option<&Path>,
    direction: &str,
) -> serde_json::Value {
    let word = if matches!(view, View::Last) { "last" } else { "files" };
    let mut command: Vec<String> =
        [exe.to_string_lossy().as_ref(), word, "--target", target, "--session", session]
            .map(str::to_owned)
            .into();
    if let View::Files(paths) = view
        && !paths.is_empty()
    {
        command.push("--".to_owned());
        command.extend(paths.iter().cloned());
    }
    let mut options = serde_json::Map::new();
    options.insert("command".to_owned(), serde_json::json!(command));
    if let Some(cwd) = cwd {
        options.insert("cwd".to_owned(), serde_json::Value::from(cwd.to_string_lossy().as_ref()));
    }
    serde_json::json!({
        "anchor_id": target,
        "direction": direction,
        "side": "after",
        "ratio": 0.5,
        "focus": true,
        "layout": {
            "block": { "flavor": TERMINAL_FLAVOR, "label": view.label(target), "options": options },
        },
    })
}

/// The Rex calls that `open_last` needs. `Rex` implements it. Tests use a fake.
pub trait OpenApi {
    fn process(&self, block: &str) -> Result<ProcessInfo, RexError>;
    fn program_status(&self, block: &str) -> Result<StatusList, RexError>;
    fn size(&self, block: &str) -> Result<Size, RexError>;
    fn view(&self) -> Result<SessionView, RexError>;
    fn new_split(&self, request: &serde_json::Value) -> Result<Vec<String>, RexError>;
    fn focus_block(&self, block: &str) -> Result<(), RexError>;
}

impl<R: Runner> OpenApi for Rex<R> {
    fn process(&self, block: &str) -> Result<ProcessInfo, RexError> {
        Rex::process(self, block)
    }
    fn program_status(&self, block: &str) -> Result<StatusList, RexError> {
        Rex::program_status(self, block)
    }
    fn size(&self, block: &str) -> Result<Size, RexError> {
        Rex::size(self, block)
    }
    fn view(&self) -> Result<SessionView, RexError> {
        Rex::view(self)
    }
    fn new_split(&self, request: &serde_json::Value) -> Result<Vec<String>, RexError> {
        Rex::new_split(self, request)
    }
    fn focus_block(&self, block: &str) -> Result<(), RexError> {
        Rex::focus_block(self, block)
    }
}

/// The viewer pane after `open_last`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Opened {
    pub block_id: String,
    pub focused_existing: bool,
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

/// Open the viewer for the agent in `target`. If a viewer for `target` is open, focus it.
pub fn open_last(
    api: &impl OpenApi,
    target: &str,
    session: &str,
    exe: &Path,
    placement: Placement,
) -> Result<Opened, OpenError> {
    open_view(api, &View::Last, target, session, exe, placement)
}

/// Open the files viewer for the agent in `target`, with `paths` open. If a files viewer for
/// `target` is open, focus it. Then the paths are not used.
pub fn open_files(
    api: &impl OpenApi,
    target: &str,
    session: &str,
    exe: &Path,
    placement: Placement,
    paths: &[String],
) -> Result<Opened, OpenError> {
    open_view(api, &View::Files(paths), target, session, exe, placement)
}

/// Open the viewer of `view` for the agent in `target`, or focus the one that is open.
pub fn open_view(
    api: &impl OpenApi,
    view: &View<'_>,
    target: &str,
    session: &str,
    exe: &Path,
    placement: Placement,
) -> Result<Opened, OpenError> {
    let process = api.process(target)?;
    let status = api.program_status(target)?;
    if detect(&process, &status).is_none() {
        return Err(OpenError::NotAgent);
    }
    if let Some(existing) = api.view()?.find_label(&view.label(target)) {
        api.focus_block(&existing.block_id)?;
        return Ok(Opened { block_id: existing.block_id.clone(), focused_existing: true });
    }
    let size = api.size(target)?;
    let cwd = process.foreground.as_ref().and_then(|fg| fg.cwd.as_deref());
    let request = split_request_for(view, target, session, exe, cwd, direction(placement, size.columns));
    let block_id = api.new_split(&request)?.into_iter().next().ok_or(OpenError::NoNewBlock)?;
    Ok(Opened { block_id, focused_existing: false })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use super::*;

    const TARGET: &str = "block:01a1b000-0000-7000-8000-000000000004";

    struct Fake {
        process: &'static str,
        status: &'static str,
        view: &'static str,
        columns: u16,
        calls: RefCell<Vec<String>>,
        split: RefCell<Option<serde_json::Value>>,
    }

    impl Fake {
        fn claude(view: &'static str, columns: u16) -> Self {
            Self {
                process: include_str!("../tests/fixtures/rex/process-claude.json"),
                status: include_str!("../tests/fixtures/rex/status-claude-working.json"),
                view,
                columns,
                calls: RefCell::default(),
                split: RefCell::default(),
            }
        }
        fn called(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    const AGENT_ONLY: &str = include_str!("../tests/fixtures/rex/view-agent-only.json");
    const WITH_ANNOREX: &str = include_str!("../tests/fixtures/rex/view-with-annorex.json");

    impl OpenApi for Fake {
        fn process(&self, _: &str) -> Result<ProcessInfo, RexError> {
            self.calls.borrow_mut().push("process".into());
            Ok(serde_json::from_str(self.process).expect("process"))
        }
        fn program_status(&self, _: &str) -> Result<StatusList, RexError> {
            self.calls.borrow_mut().push("program_status".into());
            Ok(serde_json::from_str(self.status).expect("status"))
        }
        fn size(&self, _: &str) -> Result<Size, RexError> {
            self.calls.borrow_mut().push("size".into());
            Ok(Size { columns: self.columns, rows: 40 })
        }
        fn view(&self) -> Result<SessionView, RexError> {
            self.calls.borrow_mut().push("view".into());
            Ok(serde_json::from_str(self.view).expect("view"))
        }
        fn new_split(&self, request: &serde_json::Value) -> Result<Vec<String>, RexError> {
            self.calls.borrow_mut().push("new_split".into());
            *self.split.borrow_mut() = Some(request.clone());
            Ok(vec!["block:new".into()])
        }
        fn focus_block(&self, block: &str) -> Result<(), RexError> {
            self.calls.borrow_mut().push(format!("focus_block {block}"));
            Ok(())
        }
    }

    fn exe() -> PathBuf {
        PathBuf::from("/usr/local/bin/annorex")
    }

    fn split_direction(fake: &Fake) -> String {
        fake.split.borrow().as_ref().expect("split")["direction"].as_str().expect("direction").to_owned()
    }

    #[test]
    fn auto_uses_side_by_side_at_160_columns() {
        let fake = Fake::claude(AGENT_ONLY, 160);
        open_last(&fake, TARGET, "session:s", &exe(), Placement::Auto).expect("open");
        assert_eq!(split_direction(&fake), "horizontal");
    }

    #[test]
    fn auto_uses_below_at_159_columns() {
        let fake = Fake::claude(AGENT_ONLY, 159);
        open_last(&fake, TARGET, "session:s", &exe(), Placement::Auto).expect("open");
        assert_eq!(split_direction(&fake), "vertical");
    }

    #[test]
    fn right_and_below_override_auto() {
        assert_eq!(direction(Placement::Right, 80), "horizontal");
        assert_eq!(direction(Placement::Below, 218), "vertical");
    }

    #[test]
    fn split_request_has_label_command_and_focus() {
        let fake = Fake::claude(AGENT_ONLY, 218);
        let opened = open_last(&fake, TARGET, "session:s", &exe(), Placement::Auto).expect("open");
        assert_eq!(opened, Opened { block_id: "block:new".into(), focused_existing: false });
        let request = fake.split.borrow().clone().expect("split");
        assert_eq!(request["anchor_id"], TARGET);
        assert_eq!(request["side"], "after");
        assert_eq!(request["ratio"], 0.5);
        assert_eq!(request["focus"], true);
        let block = &request["layout"]["block"];
        assert_eq!(block["flavor"], TERMINAL_FLAVOR);
        assert_eq!(block["label"], format!("annorex:{TARGET}"));
        assert_eq!(
            block["options"]["command"],
            serde_json::json!([
                "/usr/local/bin/annorex",
                "last",
                "--target",
                TARGET,
                "--session",
                "session:s"
            ])
        );
        assert_eq!(block["options"]["cwd"], "/Users/dev/work/project");
    }

    #[test]
    fn existing_annorex_pane_is_focused_not_duplicated() {
        let fake = Fake::claude(WITH_ANNOREX, 218);
        let opened = open_last(&fake, TARGET, "session:s", &exe(), Placement::Auto).expect("open");
        assert!(opened.focused_existing);
        assert_eq!(opened.block_id, "block:01a1b000-0000-7000-8000-000000000099");
        assert_eq!(
            fake.called(),
            ["process", "program_status", "view", "focus_block block:01a1b000-0000-7000-8000-000000000099"]
        );
    }

    #[test]
    fn shell_pane_is_not_agent() {
        let fake = Fake {
            process: include_str!("../tests/fixtures/rex/process-shell.json"),
            status: include_str!("../tests/fixtures/rex/status-empty.json"),
            ..Fake::claude(AGENT_ONLY, 218)
        };
        let error = open_last(&fake, TARGET, "session:s", &exe(), Placement::Auto).expect_err("not an agent");
        assert!(matches!(error, OpenError::NotAgent));
        assert_eq!(error.to_string(), "the focused pane is not running Claude Code, Pi, or OpenCode");
        assert_eq!(fake.called(), ["process", "program_status"]);
    }

    #[test]
    fn label_fits_rex_limit() {
        assert_eq!(TARGET.len(), 42);
        assert_eq!(pane_label(TARGET).len(), 50);
        assert!(pane_label(TARGET).len() <= 128);
    }

    const WITH_ANNOREX_FILES: &str = include_str!("../tests/fixtures/rex/view-with-annorex-files.json");

    fn files_command(fake: &Fake) -> Vec<String> {
        let request = fake.split.borrow().clone().expect("split");
        serde_json::from_value(request["layout"]["block"]["options"]["command"].clone()).expect("command")
    }

    #[test]
    fn split_request_for_files_has_the_files_command() {
        let request = split_request_for(&View::Files(&[]), TARGET, "session:s", &exe(), None, "vertical");
        let block = &request["layout"]["block"];
        assert_eq!(block["label"], format!("annorex-files:{TARGET}"));
        assert_eq!(
            block["options"]["command"],
            serde_json::json!([
                "/usr/local/bin/annorex",
                "files",
                "--target",
                TARGET,
                "--session",
                "session:s"
            ])
        );
    }

    #[test]
    fn open_files_passes_the_paths_to_the_split_command() {
        let fake = Fake::claude(AGENT_ONLY, 218);
        let paths = ["README.md".to_owned(), "-odd name.md".to_owned()];
        open_files(&fake, TARGET, "session:s", &exe(), Placement::Auto, &paths).expect("open");
        assert_eq!(
            files_command(&fake),
            [
                "/usr/local/bin/annorex",
                "files",
                "--target",
                TARGET,
                "--session",
                "session:s",
                "--",
                "README.md",
                "-odd name.md"
            ]
        );
    }

    #[test]
    fn open_files_uses_its_own_label_and_focuses_the_open_pane() {
        // A reply viewer for the pane does not count as the files viewer.
        let fake = Fake::claude(WITH_ANNOREX, 218);
        let opened = open_files(&fake, TARGET, "session:s", &exe(), Placement::Auto, &[]).expect("open");
        assert!(!opened.focused_existing, "the reply viewer is another pane");
        let fake = Fake::claude(WITH_ANNOREX_FILES, 218);
        let opened = open_files(&fake, TARGET, "session:s", &exe(), Placement::Auto, &[]).expect("open");
        assert!(opened.focused_existing);
        assert_eq!(
            fake.called(),
            ["process", "program_status", "view", "focus_block block:01a1b000-0000-7000-8000-000000000099"]
        );
    }

    #[test]
    fn files_command_refuses_a_pane_without_an_agent() {
        let fake = Fake {
            process: include_str!("../tests/fixtures/rex/process-shell.json"),
            status: include_str!("../tests/fixtures/rex/status-empty.json"),
            ..Fake::claude(AGENT_ONLY, 218)
        };
        let error =
            open_files(&fake, TARGET, "session:s", &exe(), Placement::Auto, &[]).expect_err("no agent");
        assert!(matches!(error, OpenError::NotAgent));
        assert_eq!(fake.called(), ["process", "program_status"]);
    }

    #[test]
    fn files_label_fits_rex_limit() {
        assert!(pane_label_files(TARGET).len() <= 128);
        assert_ne!(pane_label_files(TARGET), pane_label(TARGET));
    }
}
