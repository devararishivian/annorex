//! Live test of the full-window layer: one Annorex for each Rex session, in the windows of the
//! agents. It covers the scenarios S3, S4, S6, S9, and S10 of the design.
//!
//! The test starts the three agents but sends no prompt, so it makes no model call. It runs only
//! with `ANNOREX_LIVE=1`:
//!
//! ```text
//! ANNOREX_LIVE=1 cargo test -p annorex --test live_layer -- --ignored --nocapture
//! ```
//!
//! The viewer runs from a wrapper script with private state and config directories. The test
//! removes its Rex sessions and directories, also when it fails.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    reason = "a live test is one script that reports by printing and asserts by panicking"
)]

mod common;

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use annorex::agent::Agent;
use annorex::open::{Opened, open_last};
use annorex::rex::{Rex, SystemRunner};
use annorex_hosts::roots::Roots;
use common::{
    Cleanup, LiveAgent, accept_claude_trust, has_block, keys, live_enabled, log, start_agents,
    wait_for_state, wait_until, write_wrapper,
};

/// A session with the three agents, ready, and the handle of its Rex.
struct Live {
    session: String,
    agents: Vec<LiveAgent>,
    rex: Rex<SystemRunner>,
}

fn start(start_time: Instant, cleanup: &mut Cleanup) -> Live {
    let (session, agents) = start_agents(start_time, cleanup);
    let rex = Rex::discover_with(|key| std::env::var_os(key), Some(session.clone())).expect("rex");
    for live in &agents {
        if live.agent == Agent::ClaudeCode {
            accept_claude_trust(&rex, &session, live);
        }
    }
    for live in &agents {
        let ready = wait_for_state(&rex, &live.block, &["idle", "done"], Duration::from_secs(60));
        assert!(ready.is_some(), "{} did not become idle", live.agent.status_app());
    }
    Live { session, agents, rex }
}

fn agent(live: &Live, which: Agent) -> &LiveAgent {
    live.agents.iter().find(|candidate| candidate.agent == which).expect("agent")
}

fn open(live: &Live, block: &str, wrapper: &Path) -> Result<Opened, String> {
    open_last(&live.rex, block, &live.session, wrapper).map_err(|error| error.to_string())
}

/// The floating layers of the session that hold a block with an Annorex label.
fn annorex_layers(rex: &Rex<SystemRunner>) -> Vec<String> {
    rex.view()
        .map(|view| {
            view.locate(|block| block.label.starts_with("annorex"))
                .into_iter()
                .map(|found| found.layer.layer_id.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn api(session: &str, method: &str, args: &serde_json::Value) -> serde_json::Value {
    let output = Command::new("rex")
        .args(["api", "call", method, "--session", session, &args.to_string()])
        .output()
        .expect("rex api runs");
    assert!(output.status.success(), "{method}: {}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap_or(serde_json::Value::Null)
}

fn quit(live: &Live, viewer: &str) {
    keys(&live.rex, viewer, "q");
    let gone = wait_until(Duration::from_secs(5), || (!has_block(&live.rex, viewer)).then_some(()));
    assert!(gone.is_some(), "the viewer {viewer} did not quit");
}

#[test]
#[ignore = "uses real agents; run with ANNOREX_LIVE=1 and --ignored"]
fn one_annorex_for_each_session() {
    if !live_enabled() {
        return;
    }
    let start_time = Instant::now();
    let mut cleanup =
        Cleanup { session: None, agents: Vec::new(), roots: Roots::from_env(), dirs: Vec::new() };
    let mut cleanup_two =
        Cleanup { session: None, agents: Vec::new(), roots: Roots::from_env(), dirs: Vec::new() };
    let scratch = tempfile::Builder::new().prefix("annorex-live-layer-").tempdir().expect("scratch dir");
    let state_dir = scratch.path().join("state");
    let config_dir = scratch.path().join("xdg-config");
    for dir in [&state_dir, &config_dir] {
        std::fs::create_dir_all(dir).expect("dir");
    }
    let wrapper = write_wrapper(scratch.path(), "annorex-wrapper", &state_dir, &config_dir, None);
    cleanup.dirs.push(scratch);
    let one = start(start_time, &mut cleanup);
    let mut failures: Vec<String> = Vec::new();
    let mut check = |ok: bool, text: String| {
        log(start_time, &format!("{} {text}", if ok { "ok  " } else { "FAIL" }));
        if !ok {
            failures.push(text);
        }
    };
    let pi = agent(&one, Agent::Pi).block.clone();
    let opencode = agent(&one, Agent::OpenCode).block.clone();

    // S3: the same viewer again. No second layer; the existing one is focused.
    let first = open(&one, &pi, &wrapper).expect("open");
    check(!first.focused_existing, "S3 first open makes a new layer".to_owned());
    let again = open(&one, &pi, &wrapper).expect("open again");
    check(
        again.focused_existing && again.block_id == first.block_id && annorex_layers(&one.rex).len() == 1,
        format!("S3 second open: {again:?}, layers {:?}", annorex_layers(&one.rex)),
    );

    // S4: the viewer for Pi is open. A start on OpenCode shows the viewer for Pi and names Pi.
    let other = open(&one, &opencode, &wrapper).expect("open other");
    let names_pi = other.existing.as_ref().is_some_and(|existing| existing.agent.as_deref() == Some("pi"));
    check(
        other.focused_existing
            && other.block_id == first.block_id
            && names_pi
            && annorex_layers(&one.rex).len() == 1,
        format!("S4 start on another agent: {other:?}"),
    );
    quit(&one, &first.block_id);
    check(annorex_layers(&one.rex).is_empty(), "S4 no layer stays after quit".to_owned());

    // S10: a zoomed agent pane. The layer shows over it. Record the zoom after quit.
    let window = one.rex.window_of(&pi).expect("window");
    api(&one.session, "session.zoom_block", &serde_json::json!({ "block_id": pi }));
    let zoomed = open(&one, &pi, &wrapper).expect("open over zoom");
    let focused = wait_until(Duration::from_secs(5), || {
        let view = one.rex.view().ok()?;
        view.windows
            .iter()
            .find(|candidate| candidate.window_id == window)
            .filter(|candidate| candidate.focused_block_id.as_deref() == Some(zoomed.block_id.as_str()))
            .map(|_| ())
    });
    check(focused.is_some(), "S10 the viewer has the focus over the zoomed pane".to_owned());
    quit(&one, &zoomed.block_id);
    let zoom_after = one.rex.view().ok().and_then(|view| {
        view.windows.iter().find(|candidate| candidate.window_id == window).map(|w| w.zoomed_block_id.clone())
    });
    log(start_time, &format!("S10 zoom after quit: {zoom_after:?} (noted: Rex does not restore it)"));

    // S9: the agent is in a window that is not the active window.
    api(
        &one.session,
        "session.new_window",
        &serde_json::json!({
            "layout": { "block": { "flavor": "com.superlogical.terminal.shell", "options": { "command": ["sleep", "300"] } } },
            "focus": true
        }),
    );
    let in_background = open(&one, &pi, &wrapper).expect("open from a background window");
    let placed = one.rex.view().is_ok_and(|view| {
        view.locate(|block| block.block_id == in_background.block_id)
            .first()
            .is_some_and(|found| found.window.window_id == window)
    });
    check(placed, "S9 the layer is in the window of the agent".to_owned());
    quit(&one, &in_background.block_id);

    // S6: a second session. Each session has its own layer.
    let two = start(start_time, &mut cleanup_two);
    let pi_two = agent(&two, Agent::Pi).block.clone();
    let in_one = open(&one, &pi, &wrapper).expect("open in one");
    let in_two = open(&two, &pi_two, &wrapper).expect("open in two");
    check(
        !in_one.focused_existing
            && !in_two.focused_existing
            && annorex_layers(&one.rex).len() == 1
            && annorex_layers(&two.rex).len() == 1,
        format!("S6 each session has one layer: {in_one:?} {in_two:?}"),
    );
    quit(&one, &in_one.block_id);
    quit(&two, &in_two.block_id);

    drop(cleanup_two);
    drop(cleanup);
    log(start_time, "cleanup done");
    assert!(failures.is_empty(), "live layer failures:\n{}", failures.join("\n"));
}
