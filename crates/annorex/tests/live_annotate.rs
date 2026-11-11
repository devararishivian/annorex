//! Live test of annotating and sending on Claude Code, Pi, and `OpenCode` in a separate Rex session.
//!
//! This test uses real agents and real model calls. It runs only with `ANNOREX_LIVE=1`:
//!
//! ```text
//! ANNOREX_LIVE=1 cargo test -p annorex --test live_annotate -- --ignored --nocapture
//! ```
//!
//! The viewer runs from a wrapper script that sets `XDG_STATE_HOME` and `XDG_CONFIG_HOME` to
//! temporary directories, so the test never touches the drafts or the config of the user. The test
//! removes the Rex session, the temporary directories, and the agent sessions, also when it fails.
//! The Claude Code part saves the text clipboard with `pbpaste` and restores it at the end.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    reason = "a live test is one script that reports by printing and asserts by panicking"
)]

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use annorex::agent::Agent;
use annorex::config::Config;
use annorex::open::open_last;
use annorex::replies::{Reply, Source, candidates, load_replies, process_table};
use annorex::rex::{Rex, SystemRunner};
use annorex::safety::Safety;
use annorex::ui::{AgentState, App, Screen};
use annorex_hosts::roots::Roots;
use common::{
    ARRIVAL_TIMEOUT, Cleanup, LiveAgent, accept_claude_trust, capture, deliver, drafts, has_block,
    human_messages, keys, live_enabled, log, start_agents, unwrap_paste, wait_for_state, wait_until,
    write_wrapper,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const NOTE: &str = "ANNOREX-M2-NOTE";
const NOTE_TWO: &str = "ANNOREX-M2-NOTE-TWO";

fn heading(agent: Agent) -> String {
    let name = match agent {
        Agent::ClaudeCode => "CLAUDE",
        Agent::Pi => "PI",
        Agent::OpenCode => "OPENCODE",
    };
    format!("ANNOREX-M2-{name}")
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

/// The export that the viewer must produce for `reply` after the keys `j v l l l c <note> enter`.
fn expected_export(agent: Agent, reply: &str, note: &str) -> String {
    let state = AgentState { agent: Some(agent), safety: Safety::Clear { state: None } };
    let reply = Reply { id: "expected".to_owned(), text: reply.to_owned(), at: None };
    let mut app = App::new(state, Screen::Viewer, vec![reply], &Config::default());
    let mut press = |code: KeyCode| {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    };
    for c in "jvlllc".chars().chain(note.chars()) {
        press(KeyCode::Char(c));
    }
    press(KeyCode::Enter);
    app.export().trim().to_owned()
}

/// Open the viewer of `block` in a full-window layer, as `annorex rex open` does. The test calls
/// the library with the wrapper as the program, so that the viewer process gets the environment
/// of the wrapper. The Lua action has its own live test. The agent is focused first, as it is when
/// the user presses the key.
fn open_viewer(
    rex: &Rex<SystemRunner>,
    session: &str,
    block: &str,
    wrapper: &Path,
) -> Result<String, String> {
    rex.focus_block(block).map_err(|error| error.to_string())?;
    open_last(rex, block, session, wrapper).map(|opened| opened.block_id).map_err(|error| error.to_string())
}

/// S2: the layer of `viewer` is gone and the focus is on a block of the window of `agent`. Rex
/// chooses that block: after a layer closes, the focus goes to its own remembered block, which is
/// not always the block that `session.focus_block` set [live, 2026-11-11]. The log tells which.
fn layer_gone_and_window_focused(rex: &Rex<SystemRunner>, viewer: &str, agent: &str) -> Option<bool> {
    let view = rex.view().ok()?;
    let gone = view.blocks().all(|block| block.block_id != viewer);
    let found = view.locate(|block| block.block_id == agent);
    let window = found.first()?.window;
    let focused = window.focused_block_id.as_deref()?;
    let in_window =
        window.layers.iter().flat_map(|layer| layer.blocks.iter()).any(|block| block.block_id == focused);
    (gone && in_window).then_some(focused == agent)
}

fn pbpaste() -> Vec<u8> {
    Command::new("pbpaste").output().map(|output| output.stdout).unwrap_or_default()
}

fn pbcopy(bytes: &[u8]) {
    if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(bytes);
        }
        let _ = child.wait();
    }
}

/// Puts the text clipboard back, also when the test fails.
struct ClipboardGuard(Vec<u8>);

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        pbcopy(&self.0);
    }
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(name)).find(|path| path.exists())
}

#[test]
#[ignore = "uses real agents and model calls; run with ANNOREX_LIVE=1 and --ignored"]
fn annotate_and_send_for_each_agent() {
    if !live_enabled() {
        return;
    }
    let start = Instant::now();
    let _clipboard = ClipboardGuard(pbpaste());
    let roots = Roots::from_env();
    let mut cleanup = Cleanup { session: None, agents: Vec::new(), roots: roots.clone(), dirs: Vec::new() };
    let (session, agents) = start_agents(start, &mut cleanup);
    let scratch = tempfile::Builder::new().prefix("annorex-live-m2-").tempdir().expect("scratch dir");
    let state_dir = scratch.path().join("state");
    let config_dir = scratch.path().join("xdg-config");
    let bin_dir = scratch.path().join("bin");
    for dir in [&state_dir, &config_dir, &bin_dir] {
        std::fs::create_dir_all(dir).expect("dir");
    }
    let wrapper = write_wrapper(scratch.path(), "annorex-wrapper", &state_dir, &config_dir, None);
    for tool in ["rex", "ps"] {
        let found = which(tool).unwrap_or_else(|| panic!("{tool} is not on the PATH"));
        std::os::unix::fs::symlink(found, bin_dir.join(tool)).expect("symlink");
    }
    let bare = write_wrapper(scratch.path(), "annorex-wrapper-bare", &state_dir, &config_dir, Some(&bin_dir));
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
        let reply = newest_reply(&rex, &roots, live).expect("reply");

        // 1. Open the viewer.
        let Ok(viewer) = open_viewer(&rex, &session, &live.block, &wrapper).inspect_err(|error| {
            fail(format!("{name} open: {error}"));
        }) else {
            continue;
        };
        let shown = wait_until(Duration::from_secs(10), || {
            capture(&session, &viewer).contains(&wanted).then_some(())
        });
        if shown.is_none() {
            fail(format!("{name} show: the viewer does not show the reply:\n{}", capture(&session, &viewer)));
            continue;
        }

        // 2. Annotate and send.
        for step in ["j", "v", "lll", "c", NOTE, "\r"] {
            keys(&rex, &viewer, step);
        }
        let screen = capture(&session, &viewer);
        log(start, &format!("{name} annotated: button shown {}", screen.contains("[Send 1")));
        if !screen.contains("[Send 1") {
            fail(format!("{name} annotate: no [Send 1 ▸] button:\n{screen}"));
        }
        keys(&rex, &viewer, "S");
        let screen = capture(&session, &viewer);
        log(
            start,
            &format!("{name} send: footer after S: {}", screen.lines().last().unwrap_or_default().trim()),
        );

        // 3. The newest human message is the export.
        let expected = expected_export(live.agent, &reply, NOTE);
        let arrived = wait_until(ARRIVAL_TIMEOUT, || {
            human_messages(&rex, &roots, live).iter().any(|text| unwrap_paste(text) == expected).then_some(())
        });
        log(start, &format!("{name} send: export arrived intact {}", arrived.is_some()));
        if arrived.is_none() {
            let closest =
                human_messages(&rex, &roots, live).into_iter().max_by_key(String::len).unwrap_or_default();
            fail(format!(
                "{name} send: the export did not arrive intact.\nexpected:\n{expected}\nclosest:\n{closest}\nviewer:\n{screen}"
            ));
        }

        // 4. The draft has sent_at.
        let sent = wait_until(Duration::from_secs(10), || {
            drafts(&state_dir, live.agent).iter().any(|draft| draft["sent_at"].is_string()).then_some(())
        });
        log(start, &format!("{name} draft: sent_at set {}", sent.is_some()));
        if sent.is_none() {
            fail(format!("{name} draft: no sent_at in {:?}", drafts(&state_dir, live.agent)));
        }

        // 5. q closes the viewer without the question.
        keys(&rex, &viewer, "q");
        let gone = wait_until(Duration::from_secs(5), || (!has_block(&rex, &viewer)).then_some(()));
        log(start, &format!("{name} quit after send: viewer gone {}", gone.is_some()));
        if gone.is_none() {
            fail(format!("{name} quit: the viewer stays after a send:\n{}", capture(&session, &viewer)));
            continue;
        }
        // S2: the layer is gone and the focus is on the agent.
        let back =
            wait_until(Duration::from_secs(5), || layer_gone_and_window_focused(&rex, &viewer, &live.block));
        log(start, &format!("{name} S2: layer gone and the window has the focus; on the agent {back:?}"));
        if back.is_none() {
            fail(format!("{name} S2: the layer stays or the window has no focused pane"));
        }
        if live.agent != Agent::ClaudeCode {
            continue;
        }

        // Claude Code only: a dialog, the discard, alt+enter, copy, the quit question, and OSC 52.
        if wait_for_state(&rex, &live.block, &["idle", "done"], ARRIVAL_TIMEOUT).is_none() {
            fail(format!("{name} dialog: the agent did not finish the reply to the export"));
            continue;
        }
        let ask = "Use your AskUserQuestion tool now. Ask me one question: pick a color, Red or Blue. Do nothing else.";
        deliver(&session, &live.block, ask);
        if wait_for_state(&rex, &live.block, &["blocked"], Duration::from_secs(90)).is_none() {
            fail(format!("{name} dialog: the agent did not show a dialog"));
            continue;
        }
        let opened = open_viewer(&rex, &session, &live.block, &wrapper);
        let Some(viewer) = opened.clone().ok() else {
            fail(format!("{name} dialog: the viewer did not open"));
            continue;
        };
        // The model answered the export, so the viewer shows a newer reply. Its draft is new.
        std::thread::sleep(Duration::from_secs(1));
        let old_drafts = drafts(&state_dir, live.agent).len();
        let annotate = |note: &str| {
            for step in ["v", "lll", "c", note, "\r"] {
                keys(&rex, &viewer, step);
            }
        };
        let is_export = |text: &str, note: &str| {
            text.contains("# Annotations on last reply") && text.contains(&format!("> {note}"))
        };

        // Discard: X, y removes the draft of the shown reply.
        annotate(NOTE_TWO);
        let made = drafts(&state_dir, live.agent).len();
        keys(&rex, &viewer, "X");
        let prompt = capture(&session, &viewer).contains("Discard the notes of this reply? y/n");
        keys(&rex, &viewer, "y");
        let left = drafts(&state_dir, live.agent).len();
        log(start, &format!("{name} discard: drafts {old_drafts}->{made}->{left}, prompt {prompt}"));
        if !prompt || made != old_drafts + 1 || left != old_drafts {
            fail(format!("{name} discard: drafts {old_drafts}->{made}->{left}, prompt {prompt}"));
        }

        // alt+enter: ESC CR in one write is read as a new line.
        keys(&rex, &viewer, "c");
        keys(&rex, &viewer, "a");
        keys(&rex, &viewer, "\x1b\r");
        keys(&rex, &viewer, "b");
        let screen = capture(&session, &viewer);
        let row = |text: &str| {
            screen.lines().position(|line| line.trim_matches(|c: char| !c.is_alphanumeric()) == text)
        };
        let two_lines = matches!((row("a"), row("b")), (Some(a), Some(b)) if b > a);
        log(start, &format!("{name} alt+enter: ESC CR gives a new line {two_lines}"));
        if !two_lines {
            fail(format!("{name} alt+enter: ESC CR did not add a new line:\n{screen}"));
        }
        keys(&rex, &viewer, "\x1b");
        annotate(NOTE_TWO);

        // Copy: pbpaste gives the export.
        pbcopy(b"ANNOREX-SENTINEL");
        keys(&rex, &viewer, "Y");
        let copied = String::from_utf8_lossy(&pbpaste()).trim().to_owned();
        log(start, &format!("{name} copy: clipboard is the export {}", is_export(&copied, NOTE_TWO)));
        if !is_export(&copied, NOTE_TWO) {
            fail(format!("{name} copy: pbpaste gave {copied:?}"));
        }

        // Send during the dialog: refused.
        let before = human_messages(&rex, &roots, live).len();
        let sent_before = drafts(&state_dir, live.agent).iter().filter(|d| d["sent_at"].is_string()).count();
        keys(&rex, &viewer, "S");
        std::thread::sleep(Duration::from_secs(2));
        let screen = capture(&session, &viewer);
        let after = human_messages(&rex, &roots, live).len();
        let sent_after = drafts(&state_dir, live.agent).iter().filter(|d| d["sent_at"].is_string()).count();
        log(
            start,
            &format!(
                "{name} dialog send: Not sent shown {}, messages {before}->{after}, sent drafts {sent_before}->{sent_after}",
                screen.contains("Not sent")
            ),
        );
        if !screen.contains("Not sent") || after != before || sent_after != sent_before {
            fail(format!(
                "{name} dialog send: messages {before}->{after}, sent drafts {sent_before}->{sent_after}:\n{screen}"
            ));
        }
        rex.write(&live.block, "\x1b").expect("escape closes the dialog");

        // q asks, k keeps the draft.
        keys(&rex, &viewer, "q");
        let asked = capture(&session, &viewer).contains("Unsent notes. s send · k keep the draft");
        keys(&rex, &viewer, "k");
        let gone = wait_until(Duration::from_secs(5), || (!has_block(&rex, &viewer)).then_some(()));
        let kept = drafts(&state_dir, live.agent).iter().any(|draft| {
            draft["sent_at"].is_null()
                && draft["review"]["annotations"].as_array().is_some_and(|a| !a.is_empty())
        });
        log(
            start,
            &format!("{name} quit question: asked {asked}, gone {}, draft kept {kept}", gone.is_some()),
        );
        if !asked || gone.is_none() || !kept {
            fail(format!("{name} quit question: asked {asked}, gone {}, kept {kept}", gone.is_some()));
        }

        // OSC 52: a viewer without pbcopy on its PATH. Does Rex apply the sequence from the pane?
        if let Ok(viewer) = open_viewer(&rex, &session, &live.block, &bare) {
            std::thread::sleep(Duration::from_secs(1));
            pbcopy(b"ANNOREX-SENTINEL");
            keys(&rex, &viewer, "Y");
            let footer = capture(&session, &viewer).lines().last().unwrap_or_default().trim().to_owned();
            let clipboard = String::from_utf8_lossy(&pbpaste()).trim().to_owned();
            log(
                start,
                &format!(
                    "{name} osc52: footer {footer:?}; Rex applied OSC 52 {}",
                    is_export(&clipboard, NOTE_TWO)
                ),
            );
            keys(&rex, &viewer, "q");
            keys(&rex, &viewer, "k");
        } else {
            log(start, &format!("{name} osc52: the viewer did not open; not checked"));
        }
        rex.write(&live.block, "\x1b").ok();
    }

    drop(cleanup);
    log(start, "cleanup done");
    assert!(failures.is_empty(), "live annotate failures:\n{}", failures.join("\n"));
}
