//! Live delivery into Claude Code, Pi, and `OpenCode` in a separate Rex session.
//!
//! This test uses real agents and real model calls. It runs only with `ANNOREX_LIVE=1`:
//!
//! ```text
//! ANNOREX_LIVE=1 cargo test -p annorex --test live_delivery -- --ignored --nocapture
//! ```
//!
//! The models: Claude Code `claude-haiku-5-5`; Pi and `OpenCode`
//! `antigravity/gemini-3.8-flash-low` on `OmniRoute`. The test removes the Rex session, the
//! temporary directories, and the sessions that the agents stored for those directories, also
//! when it fails.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::indexing_slicing,
    reason = "a live test reports by printing and asserts by panicking"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use annorex::agent::Agent;
use annorex::payload::sanitize;
use annorex::rex::{Rex, SystemRunner};
use annorex_hosts::roots::Roots;
use annorex_hosts::{Role, claude, opencode, pi};

const CLAUDE_MODEL: &str = "claude-haiku-5-5";
const PI_MODEL: &str = "omni/antigravity/gemini-3.8-flash-low";
const OPENCODE_CONFIG: &str =
    r#"{"$schema":"https://opencode.ai/config.json","model":"omniroute/antigravity/gemini-3.8-flash-low"}"#;
const ARRIVAL_TIMEOUT: Duration = Duration::from_secs(120);

struct LiveAgent {
    agent: Agent,
    block: String,
    cwd: PathBuf,
}

/// Removes everything the test made, also when the test fails.
struct Cleanup {
    session: Option<String>,
    agents: Vec<(Agent, PathBuf)>,
    roots: Roots,
    dirs: Vec<tempfile::TempDir>,
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
fn opencode_sessions_in(roots: &Roots, cwd: &Path) -> Vec<String> {
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

fn log(start: Instant, text: &str) {
    eprintln!("[{:>6.1}s] {}", start.elapsed().as_secs_f64(), text.replace('\n', " / "));
}

fn rex_json(args: &[&str]) -> serde_json::Value {
    let output = Command::new("rex").args(args).output().expect("rex runs");
    assert!(output.status.success(), "rex {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).expect("rex JSON")
}

fn capture(session: &str, block: &str) -> String {
    let output = Command::new("rex")
        .args(["capture", "-s", session, "-b", block, "--trim"])
        .output()
        .expect("capture");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `annorex deliver`: the exit code.
fn deliver(session: &str, block: &str, payload: &str) -> Option<i32> {
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

fn state(rex: &Rex<SystemRunner>, block: &str) -> Option<String> {
    rex.program_status(block).ok()?.root().map(|root| root.state.clone())
}

fn wait_until<T>(timeout: Duration, mut probe: impl FnMut() -> Option<T>) -> Option<T> {
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

fn wait_for_state(
    rex: &Rex<SystemRunner>,
    block: &str,
    wanted: &[&str],
    timeout: Duration,
) -> Option<String> {
    wait_until(timeout, || state(rex, block).filter(|s| wanted.contains(&s.as_str())))
}

/// The human messages of the agent's transcript, oldest last.
fn human_messages(rex: &Rex<SystemRunner>, roots: &Roots, live: &LiveAgent) -> Vec<String> {
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
fn unwrap_paste(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("<pasted_content id=\"") else { return trimmed };
    let Some(open_end) = rest.find("\">") else { return trimmed };
    let inner = &rest[open_end + 2..];
    let Some(close) = inner.rfind("</pasted_content") else { return trimmed };
    inner[..close].trim()
}

fn arrived(rex: &Rex<SystemRunner>, roots: &Roots, live: &LiveAgent, expected: &str) -> usize {
    human_messages(rex, roots, live).iter().filter(|text| unwrap_paste(text) == expected).count()
}

/// The closest human message, for the report when the text did not arrive intact.
fn closest(rex: &Rex<SystemRunner>, roots: &Roots, live: &LiveAgent) -> String {
    human_messages(rex, roots, live).into_iter().max_by_key(String::len).unwrap_or_default()
}

fn start_agents(start: Instant, cleanup: &mut Cleanup) -> (String, Vec<LiveAgent>) {
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
fn accept_claude_trust(rex: &Rex<SystemRunner>, session: &str, live: &LiveAgent) {
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

fn busy_task(agent: Agent) -> &'static str {
    match agent {
        Agent::ClaudeCode => "Write a 1200-word essay about the history of tea. Do not use any tools.",
        Agent::Pi | Agent::OpenCode => {
            "Run the shell command `sleep 25` with your shell tool. Then reply with exactly: SLEPT"
        }
    }
}

#[test]
#[ignore = "uses real agents and model calls; run with ANNOREX_LIVE=1 and --ignored"]
fn delivery_reaches_each_agent_intact() {
    if std::env::var_os("ANNOREX_LIVE").is_none_or(|value| value != "1") {
        eprintln!("ANNOREX_LIVE is not 1: skipped");
        return;
    }
    let start = Instant::now();
    let roots = Roots::from_env();
    let mut cleanup = Cleanup { session: None, agents: Vec::new(), roots: roots.clone(), dirs: Vec::new() };
    let (session, agents) = start_agents(start, &mut cleanup);
    let rex = Rex::discover_with(|key| std::env::var_os(key), Some(session.clone())).expect("rex");
    let payload =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/payload-markdown.md"))
            .expect("payload");
    let expected = sanitize(&payload).trim().to_owned();
    let mut failures: Vec<String> = Vec::new();

    for live in &agents {
        if live.agent == Agent::ClaudeCode {
            accept_claude_trust(&rex, &session, live);
        }
    }

    for live in &agents {
        let name = live.agent.status_app();
        let mut fail = |text: String| {
            log(start, &format!("FAIL {text}"));
            failures.push(text);
        };

        // Case 1: idle.
        if wait_for_state(&rex, &live.block, &["idle", "done"], Duration::from_secs(60)).is_none() {
            fail(format!("{name} idle: the agent did not become idle"));
            continue;
        }
        let code = deliver(&session, &live.block, &payload);
        log(start, &format!("{name} idle: deliver exit {code:?}"));
        if code != Some(0) {
            fail(format!("{name} idle: exit {code:?}, expected 0"));
        }
        let count =
            wait_until(ARRIVAL_TIMEOUT, || Some(arrived(&rex, &roots, live, &expected)).filter(|n| *n >= 1));
        log(start, &format!("{name} idle: intact copies {count:?}"));
        if count != Some(1) {
            fail(format!("{name} idle: intact copies {count:?}; closest:\n{}", closest(&rex, &roots, live)));
        }

        // Case 2: busy.
        if wait_for_state(&rex, &live.block, &["idle", "done"], ARRIVAL_TIMEOUT).is_none() {
            fail(format!("{name} busy: the agent did not finish the first reply"));
            continue;
        }
        let code = deliver(&session, &live.block, busy_task(live.agent));
        log(start, &format!("{name} busy: task sent, exit {code:?}"));
        if wait_for_state(&rex, &live.block, &["working"], Duration::from_secs(30)).is_none() {
            fail(format!("{name} busy: the agent did not start to work"));
            continue;
        }
        std::thread::sleep(Duration::from_secs(4));
        let busy_state = state(&rex, &live.block);
        let code = deliver(&session, &live.block, &payload);
        log(start, &format!("{name} busy: deliver while {busy_state:?}, exit {code:?}"));
        if code != Some(0) {
            fail(format!("{name} busy: exit {code:?}, expected 0"));
        }
        let count = wait_until(Duration::from_secs(180), || {
            Some(arrived(&rex, &roots, live, &expected)).filter(|n| *n >= 2)
        });
        log(start, &format!("{name} busy: intact copies {count:?}"));
        if count != Some(2) {
            fail(format!("{name} busy: intact copies {count:?}; closest:\n{}", closest(&rex, &roots, live)));
        }

        // Case 3: dialog (Claude Code only).
        if live.agent != Agent::ClaudeCode {
            continue;
        }
        if wait_for_state(&rex, &live.block, &["idle", "done"], Duration::from_secs(180)).is_none() {
            fail(format!("{name} dialog: the agent did not finish the busy case"));
            continue;
        }
        let ask = "Use your AskUserQuestion tool now. Ask me one question: pick a color, Red or Blue. Do nothing else.";
        let code = deliver(&session, &live.block, ask);
        log(start, &format!("{name} dialog: question task sent, exit {code:?}"));
        if wait_for_state(&rex, &live.block, &["blocked"], Duration::from_secs(90)).is_none() {
            fail(format!("{name} dialog: the agent did not show a dialog"));
            continue;
        }
        let code = deliver(&session, &live.block, &payload);
        log(start, &format!("{name} dialog: deliver exit {code:?}"));
        if code != Some(3) {
            fail(format!("{name} dialog: exit {code:?}, expected 3"));
        }
        rex.write(&live.block, "\x1b").expect("escape closes the dialog");
        std::thread::sleep(Duration::from_secs(8));
        let count = arrived(&rex, &roots, live, &expected);
        let leaked = human_messages(&rex, &roots, live)
            .iter()
            .filter(|text| text.contains("Reply with exactly: ACK"))
            .count();
        log(start, &format!("{name} dialog: intact copies {count}, messages with the payload {leaked}"));
        if count != 2 || leaked != 2 {
            fail(format!("{name} dialog: the payload arrived during the dialog ({count}, {leaked})"));
        }
    }

    drop(cleanup);
    log(start, "cleanup done");
    assert!(failures.is_empty(), "live delivery failures:\n{}", failures.join("\n"));
}
