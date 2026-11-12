//! Live test of the banner for a new reply: a viewer that is open shows the banner when the agent answers.
//!
//! This test uses real agents and makes one cheap model call for each agent. It runs only with
//! `ANNOREX_LIVE=1`:
//!
//! ```text
//! ANNOREX_LIVE=1 cargo test -p annorex --test live_news -- --ignored --nocapture
//! ```
//!
//! The viewer runs from a wrapper script with private state and config directories, so the test never
//! touches the drafts or the config of the user. The test removes its Rex session, directories, and agent
//! sessions, also when it fails.

#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::too_many_lines,
    reason = "a live test is one script that reports by printing and asserts by panicking"
)]

mod common;

use std::time::{Duration, Instant};

use annorex::agent::Agent;
use annorex::open::open_files;
use annorex::rex::Rex;
use annorex_hosts::roots::Roots;
use common::{
    ARRIVAL_TIMEOUT, Cleanup, accept_claude_trust, capture, deliver, keys, live_enabled, log, start_agents,
    wait_for_state, wait_until, write_wrapper,
};

const PROMPT: &str = "Reply with exactly this one word and nothing else: ANNOREX-NEWS";

#[test]
#[ignore = "uses real agents; run with ANNOREX_LIVE=1 and --ignored"]
fn the_banner_shows_when_the_agent_answers() {
    if !live_enabled() {
        return;
    }
    let start = Instant::now();
    let roots = Roots::from_env();
    let mut cleanup = Cleanup { session: None, agents: Vec::new(), roots: roots.clone(), dirs: Vec::new() };
    let (session, agents) = start_agents(start, &mut cleanup);
    let scratch = tempfile::Builder::new().prefix("annorex-live-news-").tempdir().expect("scratch dir");
    let state_dir = scratch.path().join("state");
    let config_dir = scratch.path().join("xdg-config");
    for dir in [&state_dir, &config_dir] {
        std::fs::create_dir_all(dir).expect("dir");
    }
    let wrapper = write_wrapper(scratch.path(), "annorex-wrapper", &state_dir, &config_dir, None);
    cleanup.dirs.push(scratch);
    let rex = Rex::discover_with(|key| std::env::var_os(key), Some(session.clone())).expect("rex");
    let mut failures: Vec<String> = Vec::new();
    for live in &agents {
        if live.agent == Agent::ClaudeCode {
            accept_claude_trust(&rex, &session, live);
        }
    }
    for live in &agents {
        let name = live.agent.status_app();
        std::fs::write(live.cwd.join("n.md"), "a file\n").expect("n.md");
        if wait_for_state(&rex, &live.block, &["idle", "done"], Duration::from_secs(60)).is_none() {
            failures.push(format!("{name} setup: the agent did not become idle"));
            continue;
        }
        // The viewer opens before the agent has any reply. Then the agent answers.
        if rex.focus_block(&live.block).is_err() {
            failures.push(format!("{name} open: cannot focus the agent"));
            continue;
        }
        let paths = ["n.md".to_owned()];
        let viewer = match open_files(&rex, &live.block, &session, &wrapper, &paths) {
            Ok(opened) => opened.block_id,
            Err(error) => {
                failures.push(format!("{name} open: {error}"));
                continue;
            }
        };
        wait_until(Duration::from_secs(10), || capture(&session, &viewer).contains("a file").then_some(()));
        let code = deliver(&session, &live.block, PROMPT);
        log(start, &format!("{name}: prompt sent, exit {code:?}"));
        let banner = wait_until(ARRIVAL_TIMEOUT, || {
            capture(&session, &viewer).contains("The agent has a new reply.").then_some(())
        });
        log(start, &format!("{name} banner: shown {}", banner.is_some()));
        if banner.is_none() {
            failures.push(format!("{name} banner: none after the reply:\n{}", capture(&session, &viewer)));
            keys(&rex, &viewer, "q");
            continue;
        }
        keys(&rex, &viewer, "r");
        let gone = wait_until(Duration::from_secs(10), || {
            (!capture(&session, &viewer).contains("The agent has a new reply.")).then_some(())
        });
        log(start, &format!("{name} read again: banner gone {}", gone.is_some()));
        if gone.is_none() {
            failures.push(format!("{name} r: the banner stays:\n{}", capture(&session, &viewer)));
        }
        keys(&rex, &viewer, "q");
    }
    drop(cleanup);
    assert!(failures.is_empty(), "live news failures:\n{}", failures.join("\n"));
}
