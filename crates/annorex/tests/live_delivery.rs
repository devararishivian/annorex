//! Live delivery into Claude Code, Pi, and `OpenCode` in a separate Rex session.
//! The live harness does not start Codex yet.
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

mod common;

use std::time::{Duration, Instant};

use annorex::agent::Agent;
use annorex::payload::sanitize;
use annorex::rex::{Rex, SystemRunner};
use annorex_hosts::roots::Roots;
use common::{
    ARRIVAL_TIMEOUT, Cleanup, LiveAgent, accept_claude_trust, deliver, human_messages, live_enabled, log,
    start_agents, state, unwrap_paste, wait_for_state, wait_until,
};

fn arrived(rex: &Rex<SystemRunner>, roots: &Roots, live: &LiveAgent, expected: &str) -> usize {
    human_messages(rex, roots, live).iter().filter(|text| unwrap_paste(text) == expected).count()
}

/// The closest human message, for the report when the text did not arrive intact.
fn closest(rex: &Rex<SystemRunner>, roots: &Roots, live: &LiveAgent) -> String {
    human_messages(rex, roots, live).into_iter().max_by_key(String::len).unwrap_or_default()
}

fn busy_task(agent: Agent) -> &'static str {
    match agent {
        Agent::ClaudeCode => "Write a 1200-word essay about the history of tea. Do not use any tools.",
        Agent::Pi | Agent::OpenCode | Agent::Codex => {
            "Run the shell command `sleep 25` with your shell tool. Then reply with exactly: SLEPT"
        }
    }
}

#[test]
#[ignore = "uses real agents and model calls; run with ANNOREX_LIVE=1 and --ignored"]
fn delivery_reaches_each_agent_intact() {
    if !live_enabled() {
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
