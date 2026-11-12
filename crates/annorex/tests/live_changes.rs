//! Live test of the changed files: each agent writes a file, and the transcript reader finds it.
//!
//! This test uses real agents and makes one cheap model call for each agent. It runs only with
//! `ANNOREX_LIVE=1`:
//!
//! ```text
//! ANNOREX_LIVE=1 cargo test -p annorex --test live_changes -- --ignored --nocapture
//! ```
//!
//! The test removes its Rex session and directories, also when it fails.

#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::too_many_lines,
    reason = "a live test is one script that reports by printing and asserts by panicking"
)]

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use annorex::agent::Agent;
use annorex::changes::{normalize, read, resolve};
use annorex::replies::{Source, process_table};
use annorex::rex::Rex;
use annorex_hosts::roots::Roots;
use common::{
    Cleanup, accept_claude_trust, capture, deliver, live_enabled, log, start_agents, wait_for_state,
    wait_until,
};

const PROMPT: &str = "Create a file named m5-live.txt in the current directory with the single word hello. \
Use your file writing tool, not a shell command. Then reply with the word done.";

#[test]
#[ignore = "uses real agents; run with ANNOREX_LIVE=1 and --ignored"]
fn each_agent_reports_the_file_it_wrote() {
    if !live_enabled() {
        return;
    }
    let start = Instant::now();
    let roots = Roots::from_env();
    let mut cleanup = Cleanup { session: None, agents: Vec::new(), roots: roots.clone(), dirs: Vec::new() };
    let (session, agents) = start_agents(start, &mut cleanup);
    let rex = Rex::discover_with(|key| std::env::var_os(key), Some(session.clone())).expect("rex");
    for live in &agents {
        if live.agent == Agent::ClaudeCode {
            accept_claude_trust(&rex, &session, live);
        }
    }
    let mut failures: Vec<String> = Vec::new();
    for live in &agents {
        let name = live.agent.status_app();
        if wait_for_state(&rex, &live.block, &["idle", "done"], Duration::from_secs(60)).is_none() {
            failures.push(format!("{name}: the agent did not become idle"));
            continue;
        }
        let code = deliver(&session, &live.block, PROMPT);
        log(start, &format!("{name}: prompt sent, exit {code:?}"));
    }
    for live in &agents {
        let name = live.agent.status_app();
        let process = rex.process(&live.block).expect("process");
        let source = Source::from_process(live.agent, &process).expect("source");
        let found = wait_until(Duration::from_secs(300), || {
            let table = process_table().unwrap_or_default();
            let session_ref = resolve(&source, &roots, &table).ok()?;
            let raw = read(&session_ref, 1).ok()?;
            let files = normalize(&live.cwd, &raw);
            files.contains(&PathBuf::from("m5-live.txt")).then_some(files)
        });
        log(start, &format!("{name}: changed files {found:?}"));
        if found.is_none() {
            let table = process_table().unwrap_or_default();
            let resolved = resolve(&source, &roots, &table);
            let raw = resolved.as_ref().map(|s| read(s, 20));
            log(
                start,
                &format!(
                    "{name} debug: file on disk {}, resolved {resolved:?}, raw {raw:?}\npane:\n{}",
                    live.cwd.join("m5-live.txt").exists(),
                    capture(&session, &live.block)
                ),
            );
            failures.push(format!("{name}: m5-live.txt is not in the changed files"));
        }
    }
    drop(cleanup);
    assert!(failures.is_empty(), "live changes failures:\n{}", failures.join("\n"));
}
