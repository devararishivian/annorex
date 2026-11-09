//! Helpers of the live tests: the Rex session, the three agents, and the cleanup.
//!
//! The models: Claude Code `claude-haiku-5-5`; Pi and `OpenCode`
//! `antigravity/gemini-3.8-flash-low` on `OmniRoute`. The cleanup removes the Rex session, the
//! temporary directories, and the sessions that the agents stored for those directories, also
//! when a test fails.

#![allow(
    dead_code,
    clippy::expect_used,
    clippy::print_stderr,
    clippy::indexing_slicing,
    reason = "each live test uses a part of the helpers; a live test reports by printing and asserts by panicking"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use annorex::agent::Agent;
use annorex::rex::{Rex, SystemRunner};
use annorex_hosts::claude;
use annorex_hosts::pi;
use annorex_hosts::roots::Roots;
use annorex_hosts::{Role, opencode};

pub(crate) const CLAUDE_MODEL: &str = "claude-haiku-5-5";
pub(crate) const PI_MODEL: &str = "omni/antigravity/gemini-3.8-flash-low";
pub(crate) const OPENCODE_CONFIG: &str =
    r#"{"$schema":"https://opencode.ai/config.json","model":"omniroute/antigravity/gemini-3.8-flash-low"}"#;
pub(crate) const ARRIVAL_TIMEOUT: Duration = Duration::from_secs(120);

/// True when `ANNOREX_LIVE=1`. Else the test prints that it is skipped.
pub(crate) fn live_enabled() -> bool {
    let enabled = std::env::var_os("ANNOREX_LIVE").is_some_and(|value| value == "1");
    if !enabled {
        eprintln!("ANNOREX_LIVE is not 1: skipped");
    }
    enabled
}

pub(crate) struct LiveAgent {
    pub agent: Agent,
    pub block: String,
    pub cwd: PathBuf,
}

/// Removes everything the test made, also when the test fails.
pub(crate) struct Cleanup {
    pub session: Option<String>,
    pub agents: Vec<(Agent, PathBuf)>,
    pub roots: Roots,
    pub dirs: Vec<tempfile::TempDir>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(session) = &self.session {
            let _ = Command::new("rex")
                .args(["kill", session])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        for (agent, cwd) in &self.agents {
            match agent {
                Agent::ClaudeCode => {
                    let dir = self.roots.claude_config().join("projects").join(claude::project_slug(cwd));
                    let _ = std::fs::remove_dir_all(dir);
                }
                Agent::Pi => {
                    let _ = std::fs::remove_dir_all(self.roots.pi_sessions().join(pi::encoded_dir(cwd)));
                }
                Agent::OpenCode => {
                    for id in opencode_sessions_in(&self.roots, cwd) {
                        let _ = Command::new("opencode")
                            .args(["session", "delete", &id])
                            .current_dir(cwd)
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status();
                    }
                }
            }
        }
    }
}

/// The ids of the `OpenCode` sessions whose directory is exactly `cwd`. An exact match only:
/// the cleanup must never delete a session of the user.
pub(crate) fn opencode_sessions_in(roots: &Roots, cwd: &Path) -> Vec<String> {
    let mut ids = Vec::new();
    let dir = cwd.to_string_lossy().into_owned();
    for db in roots.opencode_databases() {
        let Ok(connection) =
            rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        else {
            continue;
        };
        for table in ["session", "session_v2"] {
            let sql = format!("SELECT id FROM {table} WHERE directory = ?1");
            let Ok(mut statement) = connection.prepare(&sql) else { continue };
            if let Ok(rows) = statement.query_map([&dir], |row| row.get::<_, String>(0)) {
                ids.extend(rows.filter_map(Result::ok));
            }
        }
    }
    ids
}

pub(crate) fn log(start: Instant, text: &str) {
    eprintln!("[{:>6.1}s] {}", start.elapsed().as_secs_f64(), text.replace('\n', " / "));
}

pub(crate) fn rex_json(args: &[&str]) -> serde_json::Value {
    let output = Command::new("rex").args(args).output().expect("rex runs");
    assert!(output.status.success(), "rex {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).expect("rex JSON")
}

pub(crate) fn capture(session: &str, block: &str) -> String {
    let output = Command::new("rex")
        .args(["capture", "-s", session, "-b", block, "--trim"])
        .output()
        .expect("capture");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `annorex deliver`: the exit code.
pub(crate) fn deliver(session: &str, block: &str, payload: &str) -> Option<i32> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_annorex"))
        .args(["deliver", "--session", session, "--target", block])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("annorex starts");
    std::io::Write::write_all(&mut child.stdin.take().expect("stdin"), payload.as_bytes()).expect("payload");
    let output = child.wait_with_output().expect("annorex ends");
    if !output.status.success() {
        eprintln!("    annorex stderr: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    output.status.code()
}

pub(crate) fn state(rex: &Rex<SystemRunner>, block: &str) -> Option<String> {
    rex.program_status(block).ok()?.root().map(|root| root.state.clone())
}

pub(crate) fn wait_until<T>(timeout: Duration, mut probe: impl FnMut() -> Option<T>) -> Option<T> {
    let end = Instant::now() + timeout;
    loop {
        if let Some(value) = probe() {
            return Some(value);
        }
        if Instant::now() >= end {
            return None;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

pub(crate) fn wait_for_state(
    rex: &Rex<SystemRunner>,
    block: &str,
    wanted: &[&str],
    timeout: Duration,
) -> Option<String> {
    wait_until(timeout, || state(rex, block).filter(|s| wanted.contains(&s.as_str())))
}

pub(crate) fn start_agents(start: Instant, cleanup: &mut Cleanup) -> (String, Vec<LiveAgent>) {
    let mut dirs = Vec::new();
    for agent in [Agent::ClaudeCode, Agent::Pi, Agent::OpenCode] {
        let dir = tempfile::Builder::new().prefix("annorex-live-").tempdir().expect("temp dir");
        let cwd = dir.path().canonicalize().expect("canonical dir");
        cleanup.agents.push((agent, cwd.clone()));
        dirs.push((agent, cwd));
        cleanup.dirs.push(dir);
    }
    let command = |agent: Agent, cwd: &Path| -> Vec<String> {
        match agent {
            Agent::ClaudeCode => vec!["claude".into(), "--model".into(), CLAUDE_MODEL.into()],
            Agent::Pi => vec!["pi".into(), "--model".into(), PI_MODEL.into()],
            Agent::OpenCode => {
                std::fs::write(cwd.join("opencode.json"), OPENCODE_CONFIG).expect("opencode.json");
                vec!["opencode".into()]
            }
        }
    };
    let (first_agent, first_cwd) = dirs[0].clone();
    let cwd_text = first_cwd.to_string_lossy().into_owned();
    let mut args: Vec<String> =
        ["new", "annorex-live", "--json", "-c", &cwd_text, "--"].map(String::from).to_vec();
    args.extend(command(first_agent, &first_cwd));
    let created = rex_json(&args.iter().map(String::as_str).collect::<Vec<_>>());
    let session = created["session_id"].as_str().expect("session id").to_owned();
    cleanup.session = Some(session.clone());
    let anchor = created["initial_windows"][0]["block_ids"][0].as_str().expect("block id").to_owned();
    let mut agents = vec![LiveAgent { agent: first_agent, block: anchor.clone(), cwd: first_cwd }];
    for (agent, cwd) in dirs.into_iter().skip(1) {
        let cwd_text = cwd.to_string_lossy().into_owned();
        let mut args: Vec<String> =
            ["split", "-s", &session, "-b", &anchor, "-c", &cwd_text, "--focus=false", "--json", "--"]
                .map(String::from)
                .to_vec();
        args.extend(command(agent, &cwd));
        let split = rex_json(&args.iter().map(String::as_str).collect::<Vec<_>>());
        let block = split["block_ids"][0].as_str().expect("block id").to_owned();
        agents.push(LiveAgent { agent, block, cwd });
    }
    log(start, &format!("started session {session}"));
    (session, agents)
}

/// Claude Code asks whether to trust a new folder. The dialog has no status record, so
/// accept it here: Down selects "Yes, I trust this folder".
pub(crate) fn accept_claude_trust(rex: &Rex<SystemRunner>, session: &str, live: &LiveAgent) {
    let shown = wait_until(Duration::from_secs(30), || {
        if state(rex, &live.block).is_some() {
            return Some(false);
        }
        capture(session, &live.block).contains("trust this folder").then_some(true)
    });
    if shown == Some(true) {
        rex.write(&live.block, "\x1b[B").expect("down");
        std::thread::sleep(Duration::from_millis(300));
        rex.write(&live.block, "\r").expect("enter");
    }
}

/// The human messages of the agent's transcript, oldest last.
pub(crate) fn human_messages(rex: &Rex<SystemRunner>, roots: &Roots, live: &LiveAgent) -> Vec<String> {
    let messages = match live.agent {
        Agent::ClaudeCode => {
            let Some(pid) = rex.process(&live.block).ok().and_then(|p| p.foreground).map(|fg| fg.pid) else {
                return Vec::new();
            };
            let ps = Command::new("ps").args(["-eo", "pid=,ppid="]).output().expect("ps");
            let table = claude::parse_ps(&String::from_utf8_lossy(&ps.stdout));
            let config = roots.claude_config();
            let found = claude::find_transcript(
                &config.join("sessions"),
                &config.join("projects"),
                &live.cwd,
                &table,
                pid,
            );
            found
                .and_then(|(path, _)| std::fs::read_to_string(path).ok())
                .map(|text| claude::parse_messages(&text, 1000))
                .unwrap_or_default()
        }
        Agent::Pi => pi::find_transcript(&roots.pi_sessions(), &live.cwd)
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| pi::parse_messages(&text, 1000))
            .unwrap_or_default(),
        Agent::OpenCode => roots
            .opencode_databases()
            .iter()
            .find_map(|db| {
                let found = opencode::find_session(db, &live.cwd).ok()?;
                opencode::messages_for_session(db, &found.id, found.schema, 1000).ok()
            })
            .unwrap_or_default(),
    };
    messages.into_iter().filter(|message| message.role == Role::Human).map(|message| message.text).collect()
}

/// Claude Code 2.1.319 stores a bracketed paste as `<pasted_content id="ID">TEXT</pasted_content id="ID">`
/// [live, 2026-11-09]. Compare the text inside the wrapper.
pub(crate) fn unwrap_paste(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("<pasted_content id=\"") else { return trimmed };
    let Some(open_end) = rest.find("\">") else { return trimmed };
    let inner = &rest[open_end + 2..];
    let Some(close) = inner.rfind("</pasted_content") else { return trimmed };
    inner[..close].trim()
}
