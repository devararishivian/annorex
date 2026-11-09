//! The command line: `annorex probe` and `annorex deliver`.

use std::ffi::OsString;
use std::io::{Read, Write};

use clap::{Parser, Subcommand};

use crate::agent::{Agent, BusyEffect, detect};
use crate::deliver::{DeliverError, DeliveryMode, Outcome, RealSleeper, deliver};
use crate::payload::sanitize;
use crate::rex::{Rex, RexError, SystemRunner};
use crate::safety::{Safety, check};

/// Exit code: sent, pasted, or probe complete.
pub const EXIT_OK: i32 = 0;
/// Exit code: Annorex refused to send (no agent, a dialog, no bracketed paste).
pub const EXIT_REFUSED: i32 = 3;
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
            send(&env, &target, session, mode, stdin, out)
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
            DeliverError::NoAgent | DeliverError::Blocked { .. } | DeliverError::NoBracketedPaste => {
                EXIT_REFUSED
            }
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

fn send(
    env: &impl Fn(&str) -> Option<OsString>,
    target: &str,
    session: Option<String>,
    mode: DeliveryMode,
    stdin: &mut impl Read,
    out: &mut impl Write,
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
    let rex: Rex<SystemRunner> = Rex::discover_with(env, session)?;
    let outcome = deliver(&rex, &RealSleeper, target, &payload, mode)?;
    let word = match outcome {
        Outcome::Sent => "sent",
        Outcome::Pasted => "pasted",
    };
    writeln!(out, "{word}").map_err(|error| io_failure(&error))?;
    Ok(EXIT_OK)
}
