//! The command line: `annorex probe`, `annorex deliver`, `annorex last`, and `annorex rex`.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::agent::{Agent, BusyEffect, detect};
use crate::config::Config;
use crate::deliver::{DeliverError, DeliveryMode, Outcome, RealSleeper, deliver};
use crate::draft::{DraftStore, drafts_dir};
use crate::install::{Check, InstallError, check_module, config_dir, install, run_rex_config_check};
use crate::open::{Existing, OpenError, View, open_view};
use crate::payload::sanitize;
use crate::rex::{Rex, RexError, SystemRunner};
use crate::safety::{Safety, check};
use crate::tui::{Start, run_viewer};

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
        /// Files to open at the start, relative to the work directory of the agent.
        paths: Vec<PathBuf>,
    },
    /// Set up the Rex integration.
    Rex {
        #[command(subcommand)]
        command: RexCommand,
    },
}

#[derive(Debug, Subcommand)]
enum RexCommand {
    /// Write the Lua module `annorex.lua` beside the `init.lua` of Rex. It never changes `init.lua`.
    Install {
        /// The directory. The default is the directory of the `init.lua` that Rex reads.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Open a viewer next to an agent pane. The Lua module runs this command.
    Open {
        #[command(subcommand)]
        view: OpenView,
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
    },
    /// The viewer of the files. If it is open for the target, focus it.
    Files {
        /// The agent block (pane), for example `block:...`.
        #[arg(long)]
        target: String,
        /// The Rex session of the target. The default is `$REX_SESSION`.
        #[arg(long)]
        session: Option<String>,
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
        Command::Last { target, session } => last(&env, &target, session),
        Command::Files { target, session, paths } => files(&env, &target, session, paths),
        Command::Rex { command: RexCommand::Install { dir } } => rex_install(&env, dir, out),
        Command::Rex { command: RexCommand::Open { view: OpenView::Last { target, session } } } => {
            rex_open(&env, &View::Last, &target, session, out)
        }
        Command::Rex { command: RexCommand::Open { view: OpenView::Files { target, session, paths } } } => {
            let paths: Vec<String> = paths.iter().map(|path| path.to_string_lossy().into_owned()).collect();
            rex_open(&env, &View::Files(&paths), &target, session, out)
        }
    };
    match result {
        Ok(code) => code,
        Err(failure) => {
            let _ = writeln!(err, "annorex: {}", failure.message);
            failure.code
        }
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
    Failure { code: EXIT_USAGE, message: format!("cannot write the output: {error}") }
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

fn last(
    env: &impl Fn(&str) -> Option<OsString>,
    target: &str,
    session: Option<String>,
) -> Result<i32, Failure> {
    let config = load_config(env)?;
    let store = drafts_dir(env).map(|dir| DraftStore { dir });
    let rex = Rex::discover_with(env, session)?;
    run_viewer(&rex, target, &Start::Last, &config, store).map_err(|error| Failure {
        code: EXIT_FAILURE,
        message: format!("cannot use the terminal: {error}"),
    })?;
    Ok(EXIT_OK)
}

fn files(
    env: &impl Fn(&str) -> Option<OsString>,
    target: &str,
    session: Option<String>,
    paths: Vec<PathBuf>,
) -> Result<i32, Failure> {
    let config = load_config(env)?;
    let store = drafts_dir(env).map(|dir| DraftStore { dir });
    let rex = Rex::discover_with(env, session)?;
    run_viewer(&rex, target, &Start::Files(paths), &config, store).map_err(|error| Failure {
        code: EXIT_FAILURE,
        message: format!("cannot use the terminal: {error}"),
    })?;
    Ok(EXIT_OK)
}

fn rex_install(
    env: &impl Fn(&str) -> Option<OsString>,
    dir: Option<PathBuf>,
    out: &mut impl Write,
) -> Result<i32, Failure> {
    let dir = dir.or_else(|| config_dir(env)).ok_or_else(|| Failure {
        code: EXIT_USAGE,
        message:
            "cannot find the Rex config directory: give --dir, or set REX_CONFIG, XDG_CONFIG_HOME, or HOME"
                .to_owned(),
    })?;
    // Rex runs actions with a short PATH, so the module starts this binary by its full path.
    let exe = std::env::current_exe().ok();
    let installed = install(&dir, exe.as_deref()).map_err(|error| Failure {
        code: if matches!(error, InstallError::Foreign { .. }) { EXIT_REFUSED } else { EXIT_FAILURE },
        message: error.to_string(),
    })?;
    let verb = if installed.replaced { "Updated" } else { "Wrote" };
    let mut lines = vec![
        format!("{verb} {}", installed.path.display()),
        String::new(),
        "Add these lines to init.lua:".to_owned(),
        String::new(),
        "  require(\"annorex\").setup{}".to_owned(),
        "  rex.bind(\"cmd+shift+a\", \"annorex_last\")".to_owned(),
        "  rex.bind(\"cmd+shift+f\", \"annorex_files\")".to_owned(),
    ];
    lines.push(String::new());
    lines.extend(install_note(exe.as_deref()));
    let scratch = std::env::temp_dir().join(format!("annorex-check-{}", std::process::id()));
    let checked = match crate::rex::find_binary(env) {
        Ok(rex) => match crate::install::make_scratch(&scratch) {
            Ok(()) => {
                let result = check_module(&scratch, exe.as_deref(), |init| run_rex_config_check(&rex, init));
                let _ = std::fs::remove_dir_all(&scratch);
                result
            }
            Err(_) => Check::NoScratch,
        },
        Err(_) => Check::Skipped,
    };
    lines.push(String::new());
    lines.push(checked.summary());
    lines.push(String::new());
    lines.push("Then run: rex config reload".to_owned());
    writeln!(out, "{}", lines.join("\n")).map_err(|error| io_failure(&error))?;
    Ok(if matches!(checked, Check::Failed(_)) { EXIT_FAILURE } else { EXIT_OK })
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
    if build || parts.contains(&"Cellar") {
        lines.push(
            "This path changes with each build or version. Run annorex rex install again from the installed binary."
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
    out: &mut impl Write,
) -> Result<i32, Failure> {
    // A bad config file stops here, with its message, and not later in a pane that closes at once.
    load_config(env)?;
    let rex = Rex::discover_with(env, session)?;
    let exe = std::env::current_exe().map_err(|error| Failure {
        code: EXIT_FAILURE,
        message: format!("cannot find the path of the annorex binary: {error}"),
    })?;
    let opened = open_view(&rex, view, target, rex.session(), &exe).map_err(|error| match error {
        OpenError::Rex(rex) => Failure::from(rex),
        OpenError::NotAgent => Failure { code: EXIT_REFUSED, message: error.to_string() },
        OpenError::NoNewBlock => Failure { code: EXIT_REX, message: error.to_string() },
    })?;
    let json = serde_json::to_string(&opened)
        .map_err(|error| Failure { code: EXIT_FAILURE, message: error.to_string() })?;
    writeln!(out, "{json}").map_err(|error| io_failure(&error))?;
    // The Lua action shows the first line of the error to the user.
    if let Some(message) = opened.existing.as_ref().and_then(Existing::notice) {
        return Err(Failure { code: EXIT_REFUSED, message });
    }
    Ok(EXIT_OK)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

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
    fn install_note_names_the_binary_and_warns_for_a_build_or_cellar_path() {
        let note = |path: &str| install_note(Some(Path::new(path))).join("\n");
        assert_eq!(note("/Users/me/.cargo/bin/annorex"), "Rex starts: /Users/me/.cargo/bin/annorex");
        assert!(note("/src/annorex/target/debug/annorex").contains("changes with each build"));
        assert!(note("/opt/homebrew/Cellar/annorex/0.1.0/bin/annorex").contains("changes with each build"));
        let unknown = install_note(None).join("\n");
        assert!(unknown.contains("$CARGO_HOME/bin") && unknown.contains("/opt/homebrew/bin"), "{unknown}");
    }

    #[test]
    fn sent_with_closed_output_exits_0() {
        let mut err = Vec::new();
        assert_eq!(report_sent(&mut Closed, &mut err, "sent"), EXIT_OK);
        let text = String::from_utf8(err).expect("utf8");
        assert!(text.contains("sent, but cannot write the output"), "{text}");
    }

    #[test]
    fn sent_with_working_output_prints_the_word() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(report_sent(&mut out, &mut err, "pasted"), EXIT_OK);
        assert_eq!(String::from_utf8(out).expect("utf8"), "pasted\n");
        assert_eq!(err, Vec::<u8>::new());
    }
}
