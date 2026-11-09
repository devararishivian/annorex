//! Live test of the `annorex_last` action on Claude Code, Pi, and `OpenCode` in a separate Rex
//! session.
//!
//! This test uses real agents and real model calls. It runs only with `ANNOREX_LIVE=1`:
//!
//! ```text
//! ANNOREX_LIVE=1 cargo test -p annorex --test live_viewer -- --ignored --nocapture
//! ```
//!
//! The action runs from a temporary `init.lua` with `rex do FILE --action annorex_last`. The
//! test removes the Rex session, the temporary directories, and the agent sessions, also when it
//! fails.

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
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use annorex::agent::Agent;
use annorex::open::pane_label;
use annorex::replies::{Source, candidates, load_replies, process_table};
use annorex::rex::{Rex, SystemRunner};
use annorex_hosts::roots::Roots;
use common::{
    ARRIVAL_TIMEOUT, Cleanup, LiveAgent, accept_claude_trust, capture, deliver, live_enabled, log, rex_json,
    start_agents, wait_for_state, wait_until,
};

const BIN: &str = env!("CARGO_BIN_EXE_annorex");
/// The sizes that the resize case requests.
const SIZES: [(u16, u16); 5] = [(218, 69), (40, 12), (120, 40), (30, 8), (80, 24)];
const NOT_AGENT: &str = "Annorex: the focused pane is not running Claude Code, Pi, or OpenCode";

fn heading(agent: Agent) -> String {
    let name = match agent {
        Agent::ClaudeCode => "CLAUDE",
        Agent::Pi => "PI",
        Agent::OpenCode => "OPENCODE",
    };
    format!("ANNOREX-M1-{name}")
}

fn task(agent: Agent) -> String {
    format!(
        "Reply with exactly this Markdown and nothing else:\n\n# {}\n\n- first item\n- second item\n\n```\nlet x = 1;\n```",
        heading(agent)
    )
}

/// The newest reply in the transcript of the agent, found the same way as the viewer finds it.
fn newest_reply(rex: &Rex<SystemRunner>, roots: &Roots, live: &LiveAgent) -> Option<String> {
    let process = rex.process(&live.block).ok()?;
    let source = Source::from_process(live.agent, &process)?;
    let table = process_table().ok()?;
    let found = candidates(&source, roots, &table).ok()?;
    let replies = load_replies(&found.first()?.session).ok()?;
    replies.into_iter().next().map(|reply| reply.text)
}

/// Write the module and an `init.lua` that loads it with the built binary.
fn write_init(dir: &Path) -> String {
    let output = Command::new(BIN).args(["rex", "install", "--dir"]).arg(dir).output().expect("install");
    assert!(output.status.success(), "install: {}", String::from_utf8_lossy(&output.stderr));
    let init = dir.join("init.lua");
    std::fs::write(&init, format!("require(\"annorex\").setup{{ bin = \"{BIN}\" }}\n")).expect("init.lua");
    init.to_string_lossy().into_owned()
}

fn run_action(init: &str, session: &str, block: &str) -> Output {
    Command::new("rex")
        .args(["do", init, "--action", "annorex_last", "-s", session, "-b", block])
        .output()
        .expect("rex do runs")
}

/// The blocks of the session with the label of the viewer of `target`.
fn viewers(rex: &Rex<SystemRunner>, target: &str) -> Vec<String> {
    let label = pane_label(target);
    rex.view()
        .map(|view| {
            view.blocks().filter(|block| block.label == label).map(|block| block.block_id.clone()).collect()
        })
        .unwrap_or_default()
}

fn has_block(rex: &Rex<SystemRunner>, block: &str) -> bool {
    rex.view().is_ok_and(|view| view.blocks().any(|info| info.block_id == block))
}

/// The pids of `annorex last` processes for `target`.
fn last_processes(target: &str) -> Vec<String> {
    let output = Command::new("ps").args(["-eo", "pid=,args="]).output().expect("ps");
    let needle = format!("last --target {target}");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.contains(&needle))
        .map(|line| line.trim().to_owned())
        .collect()
}

#[test]
#[ignore = "uses real agents and model calls; run with ANNOREX_LIVE=1 and --ignored"]
fn annorex_last_opens_viewer_for_each_agent() {
    if !live_enabled() {
        return;
    }
    let start = Instant::now();
    let roots = Roots::from_env();
    let mut cleanup = Cleanup { session: None, agents: Vec::new(), roots: roots.clone(), dirs: Vec::new() };
    let (session, agents) = start_agents(start, &mut cleanup);
    let config = tempfile::Builder::new().prefix("annorex-live-config-").tempdir().expect("config dir");
    let init = write_init(config.path());
    cleanup.dirs.push(config);
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
    for live in &agents {
        let name = live.agent.status_app();
        if wait_for_state(&rex, &live.block, &["idle", "done"], Duration::from_secs(60)).is_none() {
            fail(format!("{name} setup: the agent did not become idle"));
            continue;
        }
        let code = deliver(&session, &live.block, &task(live.agent));
        log(start, &format!("{name} setup: task sent, exit {code:?}"));
    }

    for live in &agents {
        let name = live.agent.status_app();
        let label = live.agent.host().label();
        let wanted = heading(live.agent);
        let replied = wait_until(ARRIVAL_TIMEOUT, || {
            newest_reply(&rex, &roots, live).filter(|text| text.contains(&wanted)).map(drop)
        });
        if replied.is_none()
            || wait_for_state(&rex, &live.block, &["idle", "done"], ARRIVAL_TIMEOUT).is_none()
        {
            fail(format!("{name} setup: no reply with {wanted}"));
            continue;
        }

        // 1. The action opens the viewer.
        let output = run_action(&init, &session, &live.block);
        log(start, &format!("{name} open: exit {:?}", output.status.code()));
        if !output.status.success() {
            fail(format!(
                "{name} open: exit {:?}: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            ));
            continue;
        }

        // 2. The viewer shows the header, the agent, and the reply.
        let Some(viewer) =
            wait_until(Duration::from_secs(10), || viewers(&rex, &live.block).into_iter().next())
        else {
            fail(format!("{name} open: no block with the label {}", pane_label(&live.block)));
            continue;
        };
        let shown = wait_until(Duration::from_secs(10), || {
            let screen = capture(&session, &viewer);
            (screen.contains("Annorex · last reply") && screen.contains(label) && screen.contains(&wanted))
                .then_some(())
        });
        if shown.is_none() {
            fail(format!("{name} show: the viewer does not show the reply:\n{}", capture(&session, &viewer)));
        }

        // 3. The action again focuses the same viewer.
        let output = run_action(&init, &session, &live.block);
        let count = viewers(&rex, &live.block).len();
        log(start, &format!("{name} again: exit {:?}, viewers {count}", output.status.code()));
        if !output.status.success() || count != 1 {
            fail(format!("{name} again: exit {:?}, viewers {count}, expected 1", output.status.code()));
        }

        // 4. Resizes. Rex applies a `resize` request at once only when the caller owns the block
        // or no client does. The Rex app owns the blocks of a new session, so a request from the
        // CLI can stay without effect. The test reads the real size and checks the screen for it.
        for (columns, rows) in SIZES {
            let resized =
                rex.block_call(&viewer, "resize", &serde_json::json!({ "columns": columns, "rows": rows }));
            std::thread::sleep(Duration::from_millis(1200));
            let size = rex.size(&viewer).map_or((0, 0), |size| (size.columns, size.rows));
            let screen = capture(&session, &viewer);
            let header = size.0 >= 40 && size.1 >= 10;
            let ok = if header { screen.contains("Annorex · ") } else { screen.contains("pane too small") };
            log(
                start,
                &format!(
                    "{name} resize {columns}x{rows}: call {:?}, real {}x{}, ok {ok}",
                    resized.as_ref().map(drop),
                    size.0,
                    size.1
                ),
            );
            if resized.is_err() || !ok {
                fail(format!(
                    "{name} resize {columns}x{rows} (real {}x{}): {resized:?}\n{screen}",
                    size.0, size.1
                ));
            }
        }

        // 5. q closes the viewer and keeps the agent.
        rex.write(&viewer, "q").expect("write q");
        let gone = wait_until(Duration::from_secs(5), || (!has_block(&rex, &viewer)).then_some(()));
        let agent_stays = has_block(&rex, &live.block);
        log(start, &format!("{name} quit: gone {}, agent stays {agent_stays}", gone.is_some()));
        if gone.is_none() || !agent_stays {
            fail(format!("{name} quit: viewer gone {}, agent stays {agent_stays}", gone.is_some()));
        }

        // 6. Closing the block ends the process.
        let output = run_action(&init, &session, &live.block);
        let reopened = wait_until(Duration::from_secs(10), || viewers(&rex, &live.block).into_iter().next());
        let Some(viewer) = reopened.filter(|_| output.status.success()) else {
            fail(format!("{name} close: the viewer did not open again"));
            continue;
        };
        wait_until(Duration::from_secs(5), || (!last_processes(&live.block).is_empty()).then_some(()));
        let closed =
            Command::new("rex").args(["block", "close", "-s", &session, &viewer]).output().expect("close");
        let left = wait_until(Duration::from_secs(5), || {
            let left = last_processes(&live.block);
            left.is_empty().then_some(())
        });
        log(
            start,
            &format!("{name} close: rex exit {:?}, process ended {}", closed.status.code(), left.is_some()),
        );
        if left.is_none() {
            fail(format!("{name} close: processes stay: {:?}", last_processes(&live.block)));
        }
    }

    // A plain shell pane.
    let split = rex_json(&["split", "-s", &session, "-b", &agents[0].block, "--focus=false", "--json"]);
    let shell = split["block_ids"][0].as_str().expect("block id").to_owned();
    std::thread::sleep(Duration::from_secs(2));
    let output = run_action(&init, &session, &shell);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    log(start, &format!("shell: exit {:?}, stderr {}", output.status.code(), stderr.trim()));
    if output.status.success() || !stderr.contains(NOT_AGENT) {
        fail(format!("shell: exit {:?}, stderr {stderr:?}, expected {NOT_AGENT:?}", output.status.code()));
    }

    drop(cleanup);
    log(start, "cleanup done");
    assert!(failures.is_empty(), "live viewer failures:\n{}", failures.join("\n"));
}
