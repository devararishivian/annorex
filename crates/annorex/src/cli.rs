//! The command line: `annorex probe`, `annorex deliver`, `annorex last`, and `annorex rex`.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};

use crate::agent::{Agent, BusyEffect, detect};
use crate::agents::{AgentPane, agent_panes};
use crate::config::Config;
use crate::deliver::{DeliverError, DeliveryMode, Outcome, RealSleeper, deliver};
use crate::draft::{DraftStore, drafts_dir};
use crate::install::{
    Check, InstallError, Refresh, check_module, config_dir, config_file, install, refresh,
    run_rex_config_check, stable_bin,
};
use crate::open::{Existing, OpenError, Opened, View, open_view};
use crate::payload::sanitize;
use crate::rex::{Rex, RexError, SystemRunner};
use crate::safety::{Safety, check};
use crate::tui::{Start, run_viewer};
use crate::update::{
    Cache, Plan, cache_path, channel, command_text, fetch_latest, plan, receipt_prefix, write_cache,
};

/// Exit code: sent, pasted, or probe complete.
pub const EXIT_OK: i32 = 0;
/// Exit code: Annorex refused to send (no agent, a dialog, no bracketed paste).
pub const EXIT_REFUSED: i32 = 3;
/// Exit code: another failure, for example a file that Annorex cannot write.
pub const EXIT_FAILURE: i32 = 1;
/// Exit code: a Rex call failed.
pub const EXIT_REX: i32 = 4;
/// Exit code: a usage error (`EX_USAGE`).
pub const EXIT_USAGE: i32 = 64;
/// Exit code of `annorex rex open last|files` when the target pane runs no agent. The Lua module
/// then asks which agent pane to open. Other commands give `EXIT_REFUSED` for no agent.
pub const EXIT_NOT_AGENT: i32 = 5;

/// Annotate the output of a coding agent in Rex and send the notes back.
#[derive(Debug, Parser)]
#[command(name = "annorex", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show the agent in a pane, its state, and whether Annorex can send to it.
    Probe {
        /// The target block (pane), for example `block:...`.
        #[arg(long)]
        target: String,
        /// The Rex session of the target. The default is `$REX_SESSION`.
        #[arg(long)]
        session: Option<String>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Send the text on standard input to the agent in a pane.
    Deliver {
        /// The target block (pane), for example `block:...`.
        #[arg(long)]
        target: String,
        /// The Rex session of the target. The default is `$REX_SESSION`.
        #[arg(long)]
        session: Option<String>,
        /// Paste the text, but do not press Enter.
        #[arg(long)]
        paste_only: bool,
    },
    /// Show the replies of the agent in a pane. `annorex rex open last` runs it in a new pane.
    Last {
        /// The agent block (pane), for example `block:...`.
        #[arg(long)]
        target: String,
        /// The Rex session of the target. The default is `$REX_SESSION`.
        #[arg(long)]
        session: Option<String>,
        /// Do not check for a newer release.
        #[arg(long)]
        no_update_check: bool,
    },
    /// Review the files of the work directory of the agent in a pane. `annorex rex open files` runs
    /// it in a new pane.
    Files {
        /// The agent block (pane), for example `block:...`.
        #[arg(long)]
        target: String,
        /// The Rex session of the target. The default is `$REX_SESSION`.
        #[arg(long)]
        session: Option<String>,
        /// Do not check for a newer release.
        #[arg(long)]
        no_update_check: bool,
        /// Files to open at the start, relative to the work directory of the agent.
        paths: Vec<PathBuf>,
    },
    /// Check for a newer release and upgrade Annorex.
    Update {
        /// Only check. Do not run the upgrade.
        #[arg(long)]
        check: bool,
    },
    /// Set up the Rex integration.
    Rex {
        #[command(subcommand)]
        command: RexCommand,
    },
}

#[derive(Debug, Subcommand)]
enum RexCommand {
    /// Write the Lua module and add the setup lines to `init.lua`. Then check `init.lua` and run
    /// `rex config reload`.
    Install {
        /// The directory of `annorex.lua` and `init.lua`. The default is the directory of the
        /// `init.lua` that Rex reads.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Write only the module. Do not change `init.lua` and do not reload Rex.
        #[arg(long)]
        no_init: bool,
    },
    /// Open a viewer next to an agent pane. The Lua module runs this command.
    Open {
        #[command(subcommand)]
        view: OpenView,
    },
    /// Print the agent panes of the session as one JSON line. The Lua module runs this command.
    Agents {
        /// The Rex session. The default is `$REX_SESSION`.
        #[arg(long)]
        session: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum OpenView {
    /// The viewer of the last reply. If it is open for the target, focus it.
    Last {
        /// The agent block (pane), for example `block:...`.
        #[arg(long)]
        target: String,
        /// The Rex session of the target. The default is `$REX_SESSION`.
        #[arg(long)]
        session: Option<String>,
        /// Do not check for a newer release.
        #[arg(long)]
        no_update_check: bool,
    },
    /// The viewer of the files. If it is open for the target, focus it.
    Files {
        /// The agent block (pane), for example `block:...`.
        #[arg(long)]
        target: String,
        /// The Rex session of the target. The default is `$REX_SESSION`.
        #[arg(long)]
        session: Option<String>,
        /// Do not check for a newer release.
        #[arg(long)]
        no_update_check: bool,
        /// Files to open at the start.
        paths: Vec<PathBuf>,
    },
}

/// Run the command line. Returns the exit code.
pub fn run(
    args: impl IntoIterator<Item = OsString>,
    env: impl Fn(&str) -> Option<OsString>,
    stdin: &mut impl Read,
    out: &mut impl Write,
    err: &mut impl Write,
) -> i32 {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let text = error.render().to_string();
            if error.use_stderr() {
                let _ = write!(err, "{text}");
                return EXIT_USAGE;
            }
            let _ = write!(out, "{text}");
            return EXIT_OK;
        }
    };
    let result = match cli.command {
        Command::Probe { target, session, json } => probe(&env, &target, session, json, out),
        Command::Deliver { target, session, paste_only } => {
            let mode = if paste_only { DeliveryMode::PasteOnly } else { DeliveryMode::Send };
            send(&env, &target, session, mode, stdin, out, err)
        }
        Command::Last { target, session, no_update_check } => {
            viewer(&env, &target, session, &Start::Last, no_update_check)
        }
        Command::Files { target, session, no_update_check, paths } => {
            viewer(&env, &target, session, &Start::Files(paths), no_update_check)
        }
        Command::Update { check } => update(&env, check, out, err),
        Command::Rex { command: RexCommand::Install { dir, no_init } } => {
            rex_install(&env, dir, no_init, out)
        }
        Command::Rex {
            command: RexCommand::Open { view: OpenView::Last { target, session, no_update_check } },
        } => rex_open(&env, &View::Last, &target, session, no_update_check, out),
        Command::Rex {
            command: RexCommand::Open { view: OpenView::Files { target, session, no_update_check, paths } },
        } => {
            let paths: Vec<String> = paths.iter().map(|path| path.to_string_lossy().into_owned()).collect();
            rex_open(&env, &View::Files(&paths), &target, session, no_update_check, out)
        }
        Command::Rex { command: RexCommand::Agents { session } } => rex_agents(&env, session, out),
    };
    match result {
        Ok(code) => code,
        Err(failure) => {
            let _ = writeln!(err, "annorex: {}", failure.message);
            failure.code
        }
    }
}

fn update(
    env: &impl Fn(&str) -> Option<OsString>,
    check_only: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<i32, Failure> {
    let current = env!("CARGO_PKG_VERSION");
    let Some(latest) = fetch_latest() else {
        let _ = writeln!(err, "annorex: cannot check for a new release");
        return Ok(EXIT_FAILURE);
    };
    let exe = std::env::current_exe().unwrap_or_default();
    let channel = channel(&exe, receipt_prefix(env).as_deref());
    match plan(current, &latest, check_only, channel) {
        Plan::Current => {
            writeln!(out, "Annorex {current} is the newest release.").map_err(|e| io_failure(&e))?;
            Ok(EXIT_OK)
        }
        Plan::Notice(line) => {
            writeln!(out, "{line}").map_err(|e| io_failure(&e))?;
            Ok(EXIT_OK)
        }
        Plan::Run(command) => {
            // Map the path before the upgrade. Homebrew removes the old keg afterwards, and then
            // `stable_bin` cannot resolve the old path any more.
            let bin = stable_bin(&exe);
            writeln!(out, "Running: {}", command_text(channel)).map_err(|e| io_failure(&e))?;
            let _ = out.flush();
            let (program, args) = command
                .split_first()
                .ok_or_else(|| Failure { code: EXIT_FAILURE, message: "empty upgrade command".to_owned() })?;
            let status = std::process::Command::new(program)
                .args(args)
                .status()
                .map_err(|e| Failure { code: EXIT_FAILURE, message: format!("cannot run {program}: {e}") })?;
            if !status.success() {
                return Ok(status.code().unwrap_or(EXIT_FAILURE));
            }
            if let Some(path) = cache_path(env) {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let _ = write_cache(&path, &Cache { checked_at: now, latest: None });
            }
            writeln!(out, "Running: {} rex install", bin.display()).map_err(|e| io_failure(&e))?;
            let _ = out.flush();
            let line = after_upgrade(&bin, |bin| rex_install_command(bin).status());
            writeln!(out, "{line}").map_err(|e| io_failure(&e))?;
            Ok(EXIT_OK)
        }
    }
}

/// The command `<bin> rex install`.
fn rex_install_command(bin: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(bin);
    command.args(["rex", "install"]);
    command
}

/// Run `<bin> rex install` after an upgrade. The child output goes to the user. Return the closing line.
fn after_upgrade(bin: &Path, run: impl FnOnce(&Path) -> std::io::Result<std::process::ExitStatus>) -> String {
    match run(bin) {
        Ok(status) if status.success() => "Annorex is updated.".to_owned(),
        Ok(_) => "Annorex is updated, but annorex rex install failed. Run it again to see why.".to_owned(),
        Err(error) => format!(
            "Annorex is updated, but Annorex cannot run {}: {error}. Run annorex rex install yourself.",
            bin.display()
        ),
    }
}

struct Failure {
    code: i32,
    message: String,
}

impl From<RexError> for Failure {
    fn from(error: RexError) -> Self {
        let code = if matches!(error, RexError::NoSession) { EXIT_USAGE } else { EXIT_REX };
        Self { code, message: error.to_string() }
    }
}

impl From<DeliverError> for Failure {
    fn from(error: DeliverError) -> Self {
        let code = match &error {
            DeliverError::NoAgent
            | DeliverError::Blocked { .. }
            | DeliverError::EnterWithheld { .. }
            | DeliverError::NoStatus
            | DeliverError::NoBracketedPaste => EXIT_REFUSED,
            DeliverError::EmptyPayload => EXIT_USAGE,
            DeliverError::Rex(_) => EXIT_REX,
        };
        match error {
            DeliverError::Rex(rex) => Self::from(rex),
            other => Self { code, message: other.to_string() },
        }
    }
}

fn io_failure(error: &std::io::Error) -> Failure {
    Failure { code: EXIT_FAILURE, message: format!("cannot write the output: {error}") }
}

fn probe(
    env: &impl Fn(&str) -> Option<OsString>,
    target: &str,
    session: Option<String>,
    json: bool,
    out: &mut impl Write,
) -> Result<i32, Failure> {
    let rex = Rex::discover_with(env, session)?;
    let process = rex.process(target)?;
    let status = rex.program_status(target)?;
    let modes = rex.modes(target)?;
    let agent = detect(&process, &status);
    let safety = check(&status);
    let paste = modes.bracketed_paste();
    let allowed = agent.is_some() && safety.allows_send() && paste;
    let (state, kind, msg) = match &safety {
        Safety::Clear { state } => (state.clone(), None, None),
        Safety::Blocked { kind, msg } => (Some("blocked".to_owned()), kind.clone(), msg.clone()),
    };
    let effect = agent.map(|agent| match agent.busy_effect() {
        BusyEffect::Queued => ("queued", "queued"),
        BusyEffect::Steers => ("steers", "steers the current task"),
    });
    let result = if json {
        let value = serde_json::json!({
            "agent": agent.map(Agent::status_app),
            "state": state,
            "kind": kind,
            "msg": msg,
            "bracketed_paste": paste,
            "send_allowed": allowed,
            "busy_effect": effect.map(|(id, _)| id),
        });
        writeln!(out, "{value}")
    } else {
        let mut lines = vec![
            format!("agent: {}", agent.map_or("none", |agent| agent.status_app())),
            format!("state: {}", state.as_deref().unwrap_or("unknown")),
        ];
        if let Safety::Blocked { .. } = safety {
            lines.push(format!(
                "dialog: {}{}",
                kind.as_deref().unwrap_or("unknown"),
                msg.as_deref().map(|msg| format!(" ({msg})")).unwrap_or_default()
            ));
        }
        lines.push(format!("bracketed paste: {}", if paste { "on" } else { "off" }));
        lines.push(format!("send allowed: {}", if allowed { "yes" } else { "no" }));
        if state.as_deref() == Some("working")
            && let Some((_, text)) = effect
        {
            lines.push(format!("if you send now: {text}"));
        }
        writeln!(out, "{}", lines.join("\n"))
    };
    result.map_err(|error| io_failure(&error))?;
    Ok(EXIT_OK)
}

/// Load the config file. A bad file is a usage error (exit 64).
fn load_config(env: &impl Fn(&str) -> Option<OsString>) -> Result<Config, Failure> {
    Config::from_env(env).map_err(|error| Failure { code: EXIT_USAGE, message: error.to_string() })
}

fn send(
    env: &impl Fn(&str) -> Option<OsString>,
    target: &str,
    session: Option<String>,
    mode: DeliveryMode,
    stdin: &mut impl Read,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<i32, Failure> {
    let mut payload = String::new();
    stdin.read_to_string(&mut payload).map_err(|error| Failure {
        code: EXIT_USAGE,
        message: format!("cannot read the payload from standard input: {error}"),
    })?;
    // Check the payload before Rex: an empty payload is a usage error in every case.
    if sanitize(&payload).trim().is_empty() {
        return Err(DeliverError::EmptyPayload.into());
    }
    let config = load_config(env)?;
    let rex: Rex<SystemRunner> = Rex::discover_with(env, session)?;
    let outcome = deliver(&rex, &RealSleeper, target, &payload, mode, config.enter_delay)?;
    let word = match outcome {
        Outcome::Sent => "sent",
        Outcome::Pasted => "pasted",
    };
    Ok(report_sent(out, err, word))
}

/// Print the result word of a delivery. The text is already in the agent when this runs, so a
/// closed output must not turn the result into a failure: report it on `err` and give `0`.
pub fn report_sent(out: &mut impl Write, err: &mut impl Write, word: &str) -> i32 {
    if let Err(error) = writeln!(out, "{word}") {
        let _ = writeln!(err, "annorex: {word}, but cannot write the output: {error}");
    }
    EXIT_OK
}

/// `annorex last` and `annorex files`.
fn viewer(
    env: &impl Fn(&str) -> Option<OsString>,
    target: &str,
    session: Option<String>,
    start: &Start,
    no_update_check: bool,
) -> Result<i32, Failure> {
    let config = load_config(env)?;
    let store = drafts_dir(env).map(|dir| DraftStore { dir });
    let rex = Rex::discover_with(env, session)?;
    run_viewer(&rex, target, start, &config, store, env, no_update_check).map_err(|error| Failure {
        code: EXIT_FAILURE,
        message: format!("cannot use the terminal: {error}"),
    })?;
    Ok(EXIT_OK)
}

fn rex_install(
    env: &impl Fn(&str) -> Option<OsString>,
    dir: Option<PathBuf>,
    no_init: bool,
    out: &mut impl Write,
) -> Result<i32, Failure> {
    // With --dir, init.lua is in that folder. Else it is the file that Rex reads.
    let init = dir.as_ref().map(|dir| dir.join("init.lua")).or_else(|| config_file(env));
    let dir = dir.or_else(|| config_dir(env)).ok_or_else(|| Failure {
        code: EXIT_USAGE,
        message:
            "cannot find the Rex config directory: give --dir, or set REX_CONFIG, XDG_CONFIG_HOME, or HOME"
                .to_owned(),
    })?;
    // Rex runs actions with a short PATH, so the module starts this binary by its full path. A
    // Homebrew binary gets the path that stays the same across upgrades.
    let exe = std::env::current_exe().ok().map(|exe| stable_bin(&exe));
    let installed = install(&dir, exe.as_deref()).map_err(|error| Failure {
        code: if matches!(error, InstallError::Foreign { .. }) { EXIT_REFUSED } else { EXIT_FAILURE },
        message: error.to_string(),
    })?;
    let verb = if installed.replaced { "Updated" } else { "Wrote" };
    let rex = crate::rex::find_binary(env).ok();
    let mut lines = vec![format!("{verb} {}", installed.path.display()), String::new()];
    let code = if no_init {
        module_only(&mut lines, rex.as_deref(), exe.as_deref())
    } else {
        let init = init.unwrap_or_else(|| dir.join("init.lua"));
        setup_init(&mut lines, rex.as_deref(), exe.as_deref(), &init)
    };
    writeln!(out, "{}", lines.join("\n")).map_err(|error| io_failure(&error))?;
    Ok(code)
}

/// `rex install --no-init`: the module only. Print the lines to add, check the module in a private
/// scratch folder, and ask for the reload. Returns the exit code.
fn module_only(lines: &mut Vec<String>, rex: Option<&Path>, exe: Option<&Path>) -> i32 {
    lines.push("Add these lines to init.lua:".to_owned());
    lines.push(String::new());
    lines.extend(SETUP_LINES.iter().map(|line| format!("  {line}")));
    lines.push(String::new());
    lines.extend(install_note(exe));
    let scratch = std::env::temp_dir().join(format!("annorex-check-{}", std::process::id()));
    let checked = match rex {
        Some(rex) => match crate::install::make_scratch(&scratch) {
            Ok(()) => {
                let result = check_module(&scratch, exe, |init| run_rex_config_check(rex, init));
                let _ = std::fs::remove_dir_all(&scratch);
                result
            }
            Err(_) => Check::NoScratch,
        },
        None => Check::Skipped,
    };
    lines.push(String::new());
    lines.push(checked.summary());
    lines.push(String::new());
    lines.push("Then run: rex config reload".to_owned());
    if matches!(checked, Check::Failed(_)) { EXIT_FAILURE } else { EXIT_OK }
}

/// The three setup lines, without the comment line of the setup block.
const SETUP_LINES: [&str; 3] = [
    "require(\"annorex\").setup{}",
    "rex.bind(\"cmd+shift+a\", \"annorex_last\")",
    "rex.bind(\"cmd+shift+f\", \"annorex_files\")",
];

/// `rex config check` of `file` with `rex`. Without `rex`, or when it cannot run, the check is
/// skipped.
/// The text of the error is set when `rex` was found but did not start.
fn config_check(rex: Option<&Path>, file: &Path) -> (Check, Option<String>) {
    match rex.map(|rex| run_rex_config_check(rex, file)) {
        Some(Ok(probe)) if probe.success => (Check::Passed, None),
        Some(Ok(probe)) => (Check::Failed(probe.message), None),
        Some(Err(error)) => (Check::Skipped, Some(error.to_string())),
        None => (Check::Skipped, None),
    }
}

/// The line that tells why a check or a reload did not run.
fn rex_unavailable(what: &str, start_error: Option<&str>) -> String {
    match start_error {
        Some(error) => format!("Skipped {what}: Could not run rex: {error}"),
        None => format!("Skipped {what}: rex is not available."),
    }
}

/// The note for a key that `init.lua` binds already, so Annorex left out its line.
fn bound_key_note(key: &str) -> Vec<String> {
    let action = if key == crate::setup::KEY_LAST { "annorex_last" } else { "annorex_files" };
    vec![
        String::new(),
        format!("init.lua already binds {key}, so Annorex left out its line. Choose a free key and add:"),
        format!("  rex.bind(\"KEY\", \"{action}\")"),
    ]
}

/// The state of `init.lua` that has the keys `skipped` bound.
fn state_with_bound(skipped: &[&str]) -> crate::setup::InitState {
    crate::setup::InitState::NotLoaded {
        binds_a: skipped.contains(&crate::setup::KEY_LAST),
        binds_f: skipped.contains(&crate::setup::KEY_FILES),
    }
}

/// `rex install`: add the setup lines to `init.lua`, check it, and reload Rex. Returns the exit
/// code.
fn setup_init(lines: &mut Vec<String>, rex: Option<&Path>, exe: Option<&Path>, init: &Path) -> i32 {
    use crate::setup::{Change, InitState, SetupError, apply, block};
    let indented = |text: &str| text.lines().map(|line| format!("  {line}")).collect::<Vec<_>>();
    let mut checked = Check::Skipped;
    let mut start_error = None;
    let mut failed = false;
    let mut message = None;
    match apply(init, |file| {
        let (result, error) = config_check(rex, file);
        checked = result.clone();
        start_error = error;
        result
    }) {
        Ok(Change::Created) => {
            lines.push(format!("Created {} with these lines:", init.display()));
            lines.push(String::new());
            lines.extend(indented(&block(&InitState::Missing, true)));
        }
        Ok(Change::Appended { skipped }) => {
            let state = state_with_bound(&skipped);
            lines.push(format!("Added these lines to {}:", init.display()));
            lines.push(String::new());
            lines.extend(indented(&block(&state, true)));
            for key in skipped {
                lines.extend(bound_key_note(key));
            }
        }
        Ok(Change::Unchanged) => {
            lines.push("init.lua already loads Annorex. No change.".to_owned());
            // The module can be new, so check the real init.lua in place.
            (checked, start_error) = config_check(rex, init);
            if let Check::Failed(text) = &checked {
                failed = true;
                message = Some(format!("rex config check found errors in init.lua:\n{text}"));
            }
        }
        Err(SetupError::CheckFailed { message: text, state }) => {
            failed = true;
            lines.push(
                "rex config check found errors. init.lua was not changed. Add these lines by hand:"
                    .to_owned(),
            );
            lines.push(String::new());
            lines.extend(hand_list(&state));
            message = Some(text);
        }
        Err(SetupError::Io { error, state }) => {
            failed = true;
            lines.push(format!("Cannot change {}: {error}. Add these lines by hand:", init.display()));
            lines.push(String::new());
            match state {
                Some(state) => lines.extend(hand_list(&state)),
                None => lines.extend(SETUP_LINES.iter().map(|line| format!("  {line}"))),
            }
        }
    }
    lines.push(String::new());
    lines.extend(install_note(exe));
    lines.push(String::new());
    if let Some(message) = message {
        lines.push(message);
    } else if !failed {
        lines.push(match checked {
            Check::Passed => "Checked init.lua with rex config check: no errors.".to_owned(),
            _ => rex_unavailable("the check of init.lua", start_error.as_deref()),
        });
    }
    if failed {
        return EXIT_FAILURE;
    }
    // A rex that did not start would not start for the reload, so skip it.
    if start_error.is_some() {
        lines.push(rex_unavailable("rex config reload", start_error.as_deref()));
        return EXIT_OK;
    }
    // Rex does not watch init.lua. This runs from a terminal, not from a Rex action, so it waits.
    lines.push(match rex.map(crate::rex::reload_config) {
        None => rex_unavailable("rex config reload", None),
        Some(Ok(probe)) if probe.success => {
            "Ran rex config reload: Rex uses the new configuration.".to_owned()
        }
        Some(_) => "Could not reload Rex. Run: rex config reload".to_owned(),
    });
    EXIT_OK
}

/// The lines to add by hand: the setup block without its comment line and without a bound key,
/// then a note for each bound key.
fn hand_list(state: &crate::setup::InitState) -> Vec<String> {
    use crate::setup::{InitState, KEY_FILES, KEY_LAST, block};
    let mut lines: Vec<String> = block(state, true)
        .lines()
        .filter(|line| !line.starts_with("--"))
        .map(|line| format!("  {line}"))
        .collect();
    if let InitState::NotLoaded { binds_a, binds_f } = *state {
        for key in [(binds_a, KEY_LAST), (binds_f, KEY_FILES)].into_iter().filter_map(|(b, k)| b.then_some(k))
        {
            lines.extend(bound_key_note(key));
        }
    }
    lines
}

/// What `rex install` says about the binary that the module starts.
fn install_note(exe: Option<&Path>) -> Vec<String> {
    let Some(exe) = exe.and_then(Path::to_str) else {
        return vec![
            "Cannot write the path of this binary into the module. The module looks in $CARGO_HOME/bin or ~/.cargo/bin,"
                .to_owned(),
            "then /opt/homebrew/bin, /usr/local/bin, and ~/.local/bin.".to_owned(),
        ];
    };
    let mut lines = vec![format!("Rex starts: {exe}")];
    let parts: Vec<&str> = exe.split('/').collect();
    let build = parts.windows(2).any(|pair| matches!(pair, ["target", "debug" | "release"]));
    if build {
        lines.push(
            "This path changes with each build. Run annorex rex install again from the installed binary."
                .to_owned(),
        );
    }
    // `stable_bin` keeps a Cellar path only when `<prefix>/bin/annorex` is missing: an unlinked keg.
    if parts.contains(&"Cellar") {
        lines.push(
            "This path changes with each version. Run brew link annorex, then annorex rex install again."
                .to_owned(),
        );
    }
    lines
}

fn rex_open(
    env: &impl Fn(&str) -> Option<OsString>,
    view: &View<'_>,
    target: &str,
    session: Option<String>,
    no_update_check: bool,
    out: &mut impl Write,
) -> Result<i32, Failure> {
    // A bad config file stops here, with its message, and not later in a pane that closes at once.
    load_config(env)?;
    let rex = Rex::discover_with(env, session)?;
    let exe = std::env::current_exe().map_err(|error| Failure {
        code: EXIT_FAILURE,
        message: format!("cannot find the path of the annorex binary: {error}"),
    })?;
    // The viewer runs in a pane that Rex starts, without this environment. So the variable
    // goes to the viewer as the flag.
    let no_update_check = !crate::update::should_check(no_update_check, env);
    let opened = open_view(&rex, view, target, rex.session(), &exe, no_update_check);
    // A reload that an earlier open started and that failed gives its notice now, once. A failed
    // open keeps the marker for the next open.
    let failed = crate::draft::state_dir(env).map(|dir| dir.join(RELOAD_FAILED_FILE));
    let failed_before = reload_failed_before(opened.is_ok(), failed.as_deref());
    // After an upgrade, the installed module is still the old one. Refresh it after the open: the
    // reload must not come while the action runs. A failed refresh never stops the open. Without a
    // state folder, the failure of the reload cannot be recorded, so ask the user.
    let reload_failed = refreshes_module(&opened)
        && refresh_module(config_dir(env), &stable_bin(&exe), || {
            failed.as_deref().is_some_and(|failed| {
                failed.parent().is_some_and(|dir| std::fs::create_dir_all(dir).is_ok())
                    && rex.reload_config_later(failed).is_ok()
            })
        })
        .is_some();
    let opened = opened.map_err(|error| match error {
        OpenError::Rex(rex) => Failure::from(rex),
        OpenError::NotAgent { .. } => Failure { code: EXIT_NOT_AGENT, message: error.to_string() },
        OpenError::NoNewBlock => Failure { code: EXIT_REX, message: error.to_string() },
    })?;
    let json = serde_json::to_string(&opened)
        .map_err(|error| Failure { code: EXIT_FAILURE, message: error.to_string() })?;
    writeln!(out, "{json}").map_err(|error| io_failure(&error))?;
    // The Lua action shows the first line of the error to the user.
    let existing = opened.existing.as_ref().and_then(Existing::notice);
    if let Some(message) = open_notice(failed_before || reload_failed, existing) {
        return Err(Failure { code: EXIT_REFUSED, message });
    }
    Ok(EXIT_OK)
}

/// The output of `annorex rex agents`.
#[derive(serde::Serialize)]
struct AgentList {
    agents: Vec<AgentPane>,
}

/// `annorex rex agents`: the agent panes of the session, as `{"agents":[...]}` on one line.
fn rex_agents(
    env: &impl Fn(&str) -> Option<OsString>,
    session: Option<String>,
    out: &mut impl Write,
) -> Result<i32, Failure> {
    let rex = Rex::discover_with(env, session)?;
    let home = env("HOME").map(PathBuf::from).unwrap_or_default();
    let list = AgentList { agents: agent_panes(&rex, &home)? };
    let json = serde_json::to_string(&list)
        .map_err(|error| Failure { code: EXIT_FAILURE, message: error.to_string() })?;
    writeln!(out, "{json}").map_err(|error| io_failure(&error))?;
    Ok(EXIT_OK)
}

/// Whether `rex open` refreshes the module after this result of the open. After `NotAgent`, the
/// action goes on: it asks which agent pane to open, for up to minutes. A reload then would come
/// while the action runs, so the open of the chosen pane refreshes the module. Every other result
/// ends the action at once.
fn refreshes_module(opened: &Result<Opened, OpenError>) -> bool {
    !matches!(opened, Err(OpenError::NotAgent { .. }))
}

/// The file in the state folder that a failed module reload creates.
const RELOAD_FAILED_FILE: &str = "reload-failed";

/// The notice of a module reload that Rex did not do.
const RELOAD_NOTICE: &str = "updated the Annorex module; run: rex config reload";

/// Remove the marker of a failed reload. True when it was there.
fn take_reload_failed(marker: Option<&Path>) -> bool {
    marker.is_some_and(|marker| std::fs::remove_file(marker).is_ok())
}

/// After a successful open, remove the marker of a failed reload. True when it was there.
fn reload_failed_before(opened: bool, marker: Option<&Path>) -> bool {
    opened && take_reload_failed(marker)
}

/// The notice that `rex open` gives after the view is open. The Lua action shows only the first
/// line, so the two notices share one line, the reload notice first.
fn open_notice(reload: bool, existing: Option<String>) -> Option<String> {
    match (reload, existing) {
        (true, Some(existing)) => Some(format!("{RELOAD_NOTICE}; {existing}")),
        (true, None) => Some(RELOAD_NOTICE.to_owned()),
        (false, existing) => existing,
    }
}

/// Refresh the module in `dir` with `bin` when it has another version, then call `reload` to make
/// Rex read it. Returns the notice for the user when `reload` fails. Other failures give no notice:
/// the old module still works.
fn refresh_module(dir: Option<PathBuf>, bin: &Path, reload: impl FnOnce() -> bool) -> Option<String> {
    match refresh(&dir?, Some(bin)) {
        Ok(Refresh::Updated) if !reload() => Some(RELOAD_NOTICE.to_owned()),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn a_rex_that_does_not_start_is_named_and_is_not_called_unavailable() {
        let missing = Path::new("/nonexistent-annorex-test/rex");
        let (checked, error) = config_check(Some(missing), Path::new("init.lua"));
        assert_eq!(checked, Check::Skipped);
        let error = error.expect("a start error");
        let line = rex_unavailable("the check of init.lua", Some(&error));
        assert!(line.contains("Could not run rex: "), "{line}");
        assert!(!line.contains("not available"), "{line}");
        assert_eq!(config_check(None, Path::new("init.lua")), (Check::Skipped, None));
        assert!(rex_unavailable("rex config reload", None).ends_with("rex is not available."));
    }

    fn exit(code: i32) -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code << 8)
    }

    #[test]
    fn after_upgrade_runs_rex_install_of_the_new_binary() {
        let bin = PathBuf::from("/opt/homebrew/bin/annorex");
        let mut seen = None;
        let line = after_upgrade(&bin, |path| {
            seen = Some(path.to_path_buf());
            Ok(exit(0))
        });
        assert_eq!(seen, Some(bin));
        assert_eq!(line, "Annorex is updated.");
    }

    #[test]
    fn rex_install_command_is_bin_rex_install() {
        let bin = Path::new("/opt/homebrew/bin/annorex");
        let command = rex_install_command(bin);
        assert_eq!(command.get_program(), bin.as_os_str());
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["rex", "install"]);
    }

    #[test]
    fn after_upgrade_reports_a_failed_install() {
        let line = after_upgrade(Path::new("/x/annorex"), |_| Ok(exit(1)));
        assert_eq!(line, "Annorex is updated, but annorex rex install failed. Run it again to see why.");
    }

    #[test]
    fn after_upgrade_reports_a_missing_binary() {
        let line = after_upgrade(Path::new("/x/annorex"), |_| {
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        });
        assert!(line.starts_with("Annorex is updated, but Annorex cannot run /x/annorex"), "{line}");
        assert!(line.contains("annorex rex install"), "{line}");
    }

    /// A writer that always fails, like a closed pipe.
    struct Closed;

    impl Write for Closed {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
    }

    #[test]
    fn install_note_names_the_binary_and_warns_for_a_build_path() {
        let note = |path: &str| install_note(Some(Path::new(path))).join("\n");
        assert_eq!(note("/Users/me/.cargo/bin/annorex"), "Rex starts: /Users/me/.cargo/bin/annorex");
        assert!(note("/src/annorex/target/debug/annorex").contains("changes with each build"));
        assert_eq!(note("/opt/homebrew/bin/annorex"), "Rex starts: /opt/homebrew/bin/annorex");
        let unknown = install_note(None).join("\n");
        assert!(unknown.contains("$CARGO_HOME/bin") && unknown.contains("/opt/homebrew/bin"), "{unknown}");
    }

    #[test]
    fn install_note_warns_for_a_cellar_path() {
        let note = install_note(Some(Path::new("/opt/homebrew/Cellar/annorex/0.2.0/bin/annorex")));
        assert_eq!(note.len(), 2, "{note:?}");
        assert_eq!(
            note.get(1).map(String::as_str),
            Some(
                "This path changes with each version. Run brew link annorex, then annorex rex install again."
            )
        );
    }

    #[test]
    fn a_failed_open_keeps_the_reload_marker() {
        let dir = tempfile::tempdir().expect("temp dir");
        let marker = dir.path().join(RELOAD_FAILED_FILE);
        std::fs::write(&marker, "").expect("marker");
        assert!(!reload_failed_before(false, Some(&marker)));
        assert!(marker.exists(), "a failed open keeps the marker for the next open");
        assert!(reload_failed_before(true, Some(&marker)));
        assert!(!marker.exists());
    }

    /// A config folder with a module of version 0.0.1, as an earlier release wrote it.
    fn old_module() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let current = format!("(annorex {})", env!("CARGO_PKG_VERSION"));
        let old = crate::install::module_contents(None).replacen(&current, "(annorex 0.0.1)", 1);
        std::fs::write(dir.path().join("annorex.lua"), old).expect("old module");
        dir
    }

    #[test]
    fn refresh_module_reloads_rex_after_an_update() {
        let dir = old_module();
        let mut reloads = 0;
        let notice = refresh_module(Some(dir.path().to_path_buf()), Path::new("/opt/x/annorex"), || {
            reloads += 1;
            true
        });
        assert_eq!(notice, None);
        assert_eq!(reloads, 1);
        let text = std::fs::read_to_string(dir.path().join("annorex.lua")).expect("module");
        assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
        assert!(text.contains("local INSTALLED_BIN = \"/opt/x/annorex\""), "{text}");
    }

    #[test]
    fn refresh_module_asks_for_a_reload_when_rex_fails() {
        let dir = old_module();
        let notice = refresh_module(Some(dir.path().to_path_buf()), Path::new("/opt/x/annorex"), || false);
        assert_eq!(notice.as_deref(), Some("updated the Annorex module; run: rex config reload"));
    }

    #[test]
    fn refresh_module_does_not_reload_without_an_update() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut reloads = 0;
        let mut reload = || {
            reloads += 1;
            true
        };
        assert_eq!(refresh_module(Some(dir.path().to_path_buf()), Path::new("/x"), &mut reload), None);
        assert_eq!(refresh_module(None, Path::new("/x"), &mut reload), None);
        std::fs::write(dir.path().join("annorex.lua"), crate::install::module_contents(None))
            .expect("module");
        assert_eq!(refresh_module(Some(dir.path().to_path_buf()), Path::new("/x"), &mut reload), None);
        std::fs::write(dir.path().join("annorex.lua"), "-- mine\n").expect("foreign");
        assert_eq!(refresh_module(Some(dir.path().to_path_buf()), Path::new("/x"), &mut reload), None);
        assert_eq!(reloads, 0);
        assert!(!dir.path().join("init.lua").exists());
    }

    #[test]
    fn the_module_is_not_refreshed_before_the_agent_picker() {
        assert!(
            !refreshes_module(&Err(OpenError::NotAgent { program: None })),
            "the action goes on to the picker"
        );
        let opened = Opened { block_id: "block:v".into(), focused_existing: false, existing: None };
        assert!(refreshes_module(&Ok(opened)), "a successful open refreshes as before");
        assert!(refreshes_module(&Err(OpenError::NoNewBlock)), "a failed open ends the action");
        assert!(refreshes_module(&Err(OpenError::Rex(RexError::NoSession))));
    }

    #[test]
    fn take_reload_failed_removes_the_marker_once() {
        let dir = tempfile::tempdir().expect("temp dir");
        let marker = dir.path().join(RELOAD_FAILED_FILE);
        assert!(!take_reload_failed(Some(&marker)), "no marker gives no notice");
        assert!(!take_reload_failed(None));
        std::fs::write(&marker, "").expect("marker");
        assert!(take_reload_failed(Some(&marker)));
        assert!(!marker.exists(), "the marker is removed");
        assert!(!take_reload_failed(Some(&marker)), "one notice, once");
    }

    #[test]
    fn open_notice_puts_the_reload_notice_first_and_keeps_both() {
        let reload = "updated the Annorex module; run: rex config reload";
        assert_eq!(open_notice(false, None), None);
        assert_eq!(open_notice(true, None).as_deref(), Some(reload));
        assert_eq!(open_notice(false, Some("x is open".to_owned())).as_deref(), Some("x is open"));
        assert_eq!(open_notice(true, Some("x is open".to_owned())), Some(format!("{reload}; x is open")));
    }

    #[test]
    fn sent_with_closed_output_exits_0() {
        let mut err = Vec::new();
        assert_eq!(report_sent(&mut Closed, &mut err, "sent"), EXIT_OK);
        let text = String::from_utf8(err).expect("utf8");
        assert!(text.contains("sent, but cannot write the output"), "{text}");
    }

    #[test]
    fn a_failed_output_write_exits_1() {
        let error = std::io::Error::from(std::io::ErrorKind::BrokenPipe);
        let failure = io_failure(&error);
        assert_eq!(failure.code, EXIT_FAILURE);
        assert!(failure.message.starts_with("cannot write the output"), "{}", failure.message);
    }

    #[test]
    fn sent_with_working_output_prints_the_word() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(report_sent(&mut out, &mut err, "pasted"), EXIT_OK);
        assert_eq!(String::from_utf8(out).expect("utf8"), "pasted\n");
        assert_eq!(err, Vec::<u8>::new());
    }
}
