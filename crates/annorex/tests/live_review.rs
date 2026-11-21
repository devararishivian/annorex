//! Live test of the one Annorex: a note on the last reply and a note on a file go to the agent in one message.
//!
//! This test uses real agents and makes one cheap model call for each agent. It runs only with
//! `ANNOREX_LIVE=1`:
//!
//! ```text
//! ANNOREX_LIVE=1 cargo test -p annorex --test live_review -- --ignored --nocapture
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
    ARRIVAL_TIMEOUT, Cleanup, accept_claude_trust, capture, deliver, drafts, has_block, human_messages, keys,
    live_enabled, log, start_agents, unwrap_paste, wait_for_state, wait_until, write_wrapper,
};

const NOTE_REPLY: &str = "ANNOREX-M6-NOTE-REPLY";
const NOTE_FILE: &str = "ANNOREX-M6-NOTE-FILE";
const MARKER: &str = "ANNOREX-M6-REPLY";
const PROMPT: &str = "Reply with exactly this one word and nothing else: ANNOREX-M6-REPLY";
const FILE: &str = "first line of the file\nsecond line of the file\n";

#[test]
#[ignore = "uses real agents; run with ANNOREX_LIVE=1 and --ignored"]
fn one_message_holds_a_reply_note_and_a_file_note() {
    if !live_enabled() {
        return;
    }
    let start = Instant::now();
    let roots = Roots::from_env();
    let mut cleanup = Cleanup { session: None, agents: Vec::new(), roots: roots.clone(), dirs: Vec::new() };
    let (session, agents) = start_agents(start, &mut cleanup);
    let scratch = tempfile::Builder::new().prefix("annorex-live-m6-").tempdir().expect("scratch dir");
    let state_dir = scratch.path().join("state");
    let config_dir = scratch.path().join("xdg-config");
    for dir in [&state_dir, &config_dir] {
        std::fs::create_dir_all(dir).expect("dir");
    }
    let wrapper = write_wrapper(scratch.path(), "annorex-wrapper", &state_dir, &config_dir, None);
    cleanup.dirs.push(scratch);
    let rex = Rex::discover_with(|key| std::env::var_os(key), Some(session.clone())).expect("rex");
    let mut failures: Vec<String> = Vec::new();
    let mut fail = |text: String| {
        log(start, &format!("FAIL {text}"));
        failures.push(text);
    };
    for live in &agents {
        if live.agent == Agent::ClaudeCode {
            accept_claude_trust(&rex, &session, live);
        }
    }
    // 1. Every agent makes one reply.
    for live in &agents {
        let name = live.agent.status_app();
        std::fs::write(live.cwd.join("m6.md"), FILE).expect("m6.md");
        if wait_for_state(&rex, &live.block, &["idle", "done"], Duration::from_secs(60)).is_none() {
            fail(format!("{name} setup: the agent did not become idle"));
            continue;
        }
        let code = deliver(&session, &live.block, PROMPT);
        log(start, &format!("{name}: prompt sent, exit {code:?}"));
    }
    for live in &agents {
        let name = live.agent.status_app();
        if wait_for_state(&rex, &live.block, &["idle", "done"], ARRIVAL_TIMEOUT).is_none() {
            fail(format!("{name} reply: the agent did not finish"));
            continue;
        }

        // 2. Open the viewer with the file. The reply is the item before the file.
        if rex.focus_block(&live.block).is_err() {
            fail(format!("{name} open: cannot focus the agent"));
            continue;
        }
        let paths = ["m6.md".to_owned()];
        let viewer = match open_files(&rex, &live.block, &session, &wrapper, &paths) {
            Ok(opened) => opened.block_id,
            Err(error) => {
                fail(format!("{name} open: {error}"));
                continue;
            }
        };
        let shown = wait_until(Duration::from_secs(10), || {
            capture(&session, &viewer).contains("first line of the file").then_some(())
        });
        if shown.is_none() {
            fail(format!("{name} show: the viewer does not show m6.md:\n{}", capture(&session, &viewer)));
            continue;
        }

        // 3. A note on the file, then a note on the last reply.
        for step in ["0", "v", "llll", "c", NOTE_FILE, "\r", "["] {
            keys(&rex, &viewer, step);
        }
        let reply =
            wait_until(Duration::from_secs(10), || capture(&session, &viewer).contains(MARKER).then_some(()));
        if reply.is_none() {
            fail(format!(
                "{name} reply: the last reply is not on the screen:\n{}",
                capture(&session, &viewer)
            ));
            continue;
        }
        for step in ["0", "v", "ll", "c", NOTE_REPLY, "\r"] {
            keys(&rex, &viewer, step);
        }
        let screen = capture(&session, &viewer);
        if !screen.contains("[ Send 2") {
            fail(format!("{name} annotate: no [ Send 2 ▸] button:\n{screen}"));
        }
        keys(&rex, &viewer, "S");

        // 4. One message holds both notes and both sections.
        let arrived = wait_until(ARRIVAL_TIMEOUT, || {
            human_messages(&rex, &roots, live)
                .iter()
                .map(|text| unwrap_paste(text))
                .find(|text| {
                    text.contains(NOTE_REPLY)
                        && text.contains(NOTE_FILE)
                        && text.contains("# Annotations on last reply")
                        && text.contains("# Annotations on m6.md")
                })
                .map(|_| ())
        });
        log(start, &format!("{name} send: one message with both notes {}", arrived.is_some()));
        if arrived.is_none() {
            let closest =
                human_messages(&rex, &roots, live).into_iter().max_by_key(String::len).unwrap_or_default();
            fail(format!("{name} send: no message with both notes.\nclosest:\n{closest}"));
        }

        // 5. Both drafts have sent_at.
        let sent = wait_until(Duration::from_secs(10), || {
            let all = drafts(&state_dir, live.agent);
            (all.len() == 2 && all.iter().all(|draft| draft["sent_at"].is_string())).then_some(())
        });
        log(start, &format!("{name} drafts: both have sent_at {}", sent.is_some()));
        if sent.is_none() {
            fail(format!("{name} drafts: {:?}", drafts(&state_dir, live.agent)));
        }
        // A finished send closes the viewer by itself, so no key is needed.
        let gone = wait_until(Duration::from_secs(5), || (!has_block(&rex, &viewer)).then_some(()));
        if gone.is_none() {
            fail(format!("{name} quit: the viewer stays after a send:\n{}", capture(&session, &viewer)));
        }
    }
    drop(cleanup);
    log(start, "cleanup done");
    assert!(failures.is_empty(), "live review failures:\n{}", failures.join("\n"));
}
