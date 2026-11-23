//! Live test of the files review on Claude Code, Pi, and `OpenCode` in a separate Rex session.
//! The live harness does not start Codex yet.
//!
//! This test uses real agents. It runs only with `ANNOREX_LIVE=1`:
//!
//! ```text
//! ANNOREX_LIVE=1 cargo test -p annorex --test live_files -- --ignored --nocapture
//! ```
//!
//! The viewer runs from a wrapper script with private state and config directories, so the test
//! never touches the drafts or the config of the user. The test removes the Rex session, the
//! temporary directories, and the agent sessions, also when it fails.

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
use std::time::{Duration, Instant};

use annorex::agent::Agent;
use annorex::config::Config;
use annorex::files::{load, resolve};
use annorex::open::open_files;
use annorex::rex::{Rex, SystemRunner};
use annorex::safety::Safety;
use annorex::ui::{AgentState, App};
use annorex_hosts::roots::Roots;
use common::{
    ARRIVAL_TIMEOUT, Cleanup, accept_claude_trust, capture, drafts, has_block, human_messages, keys,
    live_enabled, log, start_agents, unwrap_paste, wait_for_state, wait_until, write_wrapper,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const NOTE_ONE: &str = "ANNOREX-M3-NOTE-ONE";
const NOTE_TWO: &str = "ANNOREX-M3-NOTE-TWO";
const ONE: &str = "# One\n\nfirst line of one\nsecond line of one\n";
const TWO: &str = "alpha line\nbeta line\n";

/// The keys that annotate `two.txt` (shown first, because it opened last) and then `one.md`.
/// A note ends with a Return key. The text of `one.md` shows without the `# `, so `vll` marks `One`.
const STEPS: [&str; 10] = ["0", "v", "llll", "c", NOTE_TWO, "\r", "[", "v", "ll", "c"];

/// Write the project of one agent: a `.gitignore`, two files to review, and a file that `.gitignore`
/// hides (the tree lists it).
fn write_project(cwd: &Path) {
    std::fs::write(cwd.join(".gitignore"), "ignored.txt\n").expect(".gitignore");
    std::fs::write(cwd.join("one.md"), ONE).expect("one.md");
    std::fs::write(cwd.join("two.txt"), TWO).expect("two.txt");
    std::fs::write(cwd.join("ignored.txt"), "hidden\n").expect("ignored.txt");
}

/// The export that the viewer must produce: the same keys in a model of the viewer.
fn expected_export(agent: Agent, cwd: &Path) -> String {
    let state = AgentState { agent: Some(agent), safety: Safety::Clear { state: None } };
    let mut app = App::new(state, annorex::ui::Screen::Viewer, Vec::new(), &Config::default())
        .with_root(cwd.to_path_buf());
    for name in ["one.md", "two.txt"] {
        let relative = resolve(cwd, Path::new(name)).expect("resolve");
        app.open_file(load(cwd, &relative).expect("load"));
    }
    for step in STEPS.iter().chain(&[NOTE_ONE, "\r"]) {
        for c in step.chars() {
            let code = if c == '\r' { KeyCode::Enter } else { KeyCode::Char(c) };
            app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        }
    }
    app.export().trim().to_owned()
}

/// Open the files viewer of `block` in a full-window layer, with the two files. The test calls the
/// library with the wrapper as the program, so that the viewer gets the private state directory.
fn open_viewer(
    rex: &Rex<SystemRunner>,
    session: &str,
    block: &str,
    wrapper: &Path,
) -> Result<String, String> {
    rex.focus_block(block).map_err(|error| error.to_string())?;
    let paths = ["one.md".to_owned(), "two.txt".to_owned()];
    open_files(rex, block, session, wrapper, &paths)
        .map(|opened| opened.block_id)
        .map_err(|error| error.to_string())
}

/// The drafts of the files, with and without `sent_at`.
fn file_drafts(state: &Path, agent: Agent) -> (usize, usize) {
    let all: Vec<_> = drafts(state, agent)
        .into_iter()
        .filter(|draft| draft["key"]["document"].as_str().is_some_and(|id| id.starts_with("file:")))
        .collect();
    let sent = all.iter().filter(|draft| draft["sent_at"].is_string()).count();
    (all.len(), sent)
}

#[test]
#[ignore = "uses real agents; run with ANNOREX_LIVE=1 and --ignored"]
fn review_files_for_each_agent() {
    if !live_enabled() {
        return;
    }
    let start = Instant::now();
    let roots = Roots::from_env();
    let mut cleanup = Cleanup { session: None, agents: Vec::new(), roots: roots.clone(), dirs: Vec::new() };
    let (session, agents) = start_agents(start, &mut cleanup);
    let scratch = tempfile::Builder::new().prefix("annorex-live-m3-").tempdir().expect("scratch dir");
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
    for live in &agents {
        write_project(&live.cwd);
    }

    for live in &agents {
        let name = live.agent.status_app();
        if wait_for_state(&rex, &live.block, &["idle", "done"], Duration::from_secs(60)).is_none() {
            fail(format!("{name} setup: the agent did not become idle"));
            continue;
        }

        // 1. Start the viewer with the two paths. The list has both. The tree lists ignored.txt too.
        let Ok(viewer) = open_viewer(&rex, &session, &live.block, &wrapper).inspect_err(|error| {
            fail(format!("{name} open: {error}"));
        }) else {
            continue;
        };
        let shown = wait_until(Duration::from_secs(10), || {
            capture(&session, &viewer).contains("alpha line").then_some(())
        });
        if shown.is_none() {
            fail(format!("{name} show: the viewer does not show two.txt:\n{}", capture(&session, &viewer)));
            continue;
        }
        keys(&rex, &viewer, "p");
        let list = capture(&session, &viewer);
        let listed = list.contains("one.md") && list.contains("two.txt");
        log(start, &format!("{name} list: both files listed {listed}"));
        if !listed {
            fail(format!("{name} list: one.md and two.txt are not both listed:\n{list}"));
        }
        keys(&rex, &viewer, "\x1b");
        // `esc` closed the list. The tree shows by default at 100 columns or more. In a narrower pane it
        // opens over the document with `T` (docs/guide.md, "Review files"). Since 0.2.3 it lists the
        // files that `.gitignore` hides.
        let tree_of = |screen: &str| screen.contains("two.txt") && screen.contains("ignored.txt");
        let mut tree = String::new();
        let mut shown_tree = wait_until(Duration::from_secs(5), || {
            tree = capture(&session, &viewer);
            tree_of(&tree).then_some(())
        });
        let mut opened_tree = false;
        if shown_tree.is_none() {
            opened_tree = true;
            keys(&rex, &viewer, "T");
            shown_tree = wait_until(Duration::from_secs(5), || {
                tree = capture(&session, &viewer);
                tree_of(&tree).then_some(())
            });
        }
        log(start, &format!("{name} tree: shown {}", shown_tree.is_some()));
        if opened_tree {
            // A second `T` closes the tree over the document, so the next keys go to the document.
            keys(&rex, &viewer, "T");
        }
        let all_listed = tree.contains("one.md") && tree.contains("two.txt") && tree.contains("ignored.txt");
        log(start, &format!("{name} tree: ignored.txt listed {all_listed}"));
        if !all_listed {
            fail(format!("{name} tree: the tree must list one.md, two.txt, and ignored.txt:\n{tree}"));
        }
        // No `esc` here: with nothing selected or focused it quits Annorex (docs/guide.md, key rules).

        // 2. Annotate a line in each file and send.
        for step in STEPS {
            keys(&rex, &viewer, step);
        }
        for step in [NOTE_ONE, "\r"] {
            keys(&rex, &viewer, step);
        }
        let screen = capture(&session, &viewer);
        log(start, &format!("{name} annotated: button shown {}", screen.contains("[ Send 2")));
        if !screen.contains("[ Send 2") {
            fail(format!("{name} annotate: no [ Send 2 ▸] button:\n{screen}"));
        }
        keys(&rex, &viewer, "S");

        // 3. One message with the sections in the order of opening.
        let expected = expected_export(live.agent, &live.cwd);
        let arrived = wait_until(ARRIVAL_TIMEOUT, || {
            human_messages(&rex, &roots, live).iter().any(|text| unwrap_paste(text) == expected).then_some(())
        });
        log(start, &format!("{name} send: one message arrived intact {}", arrived.is_some()));
        let order = expected.find("# Annotations on one.md").zip(expected.find("# Annotations on two.txt"));
        if arrived.is_none() || order.is_none_or(|(one, two)| one >= two) {
            let closest =
                human_messages(&rex, &roots, live).into_iter().max_by_key(String::len).unwrap_or_default();
            fail(format!(
                "{name} send: no intact export.\nexpected:\n{expected}\nclosest:\n{closest}\nviewer:\n{}",
                capture(&session, &viewer)
            ));
        }

        // 4. The send took both notes: both drafts are gone and none has sent_at.
        let cleared = wait_until(Duration::from_secs(10), || {
            let (all, sent) = file_drafts(&state_dir, live.agent);
            (all == 0 && sent == 0).then_some(())
        });
        log(start, &format!("{name} drafts: both cleared after the send {}", cleared.is_some()));
        if cleared.is_none() {
            fail(format!("{name} drafts: {:?}", file_drafts(&state_dir, live.agent)));
        }
        // A finished send closes the viewer by itself, so no key is needed.
        let gone = wait_until(Duration::from_secs(5), || (!has_block(&rex, &viewer)).then_some(()));
        log(start, &format!("{name} quit after send: viewer gone {}", gone.is_some()));
        if gone.is_none() {
            fail(format!("{name} quit: the viewer stays after a send:\n{}", capture(&session, &viewer)));
            continue;
        }

        // 5. Open again: the files start clean. Add a new note on two.txt, change the file, and wait for the
        //    banner. r clears it, the new note follows.
        if wait_for_state(&rex, &live.block, &["idle", "done"], ARRIVAL_TIMEOUT).is_none() {
            fail(format!("{name} banner: the agent did not finish the reply to the export"));
            continue;
        }
        let Ok(viewer) = open_viewer(&rex, &session, &live.block, &wrapper).inspect_err(|error| {
            fail(format!("{name} open: {error}"));
        }) else {
            continue;
        };
        wait_until(Duration::from_secs(10), || {
            capture(&session, &viewer).contains("alpha line").then_some(())
        });
        let reopened = capture(&session, &viewer);
        let is_clean = !reopened.contains("[ Sent") && !reopened.contains(NOTE_TWO);
        log(start, &format!("{name} reopen: no sent label and no old note {is_clean}"));
        if !is_clean {
            fail(format!("{name} reopen: the sent notes came back:\n{reopened}"));
        }
        for step in ["0", "v", "llll", "c", NOTE_TWO, "\r"] {
            keys(&rex, &viewer, step);
        }
        std::fs::write(live.cwd.join("two.txt"), format!("a new first line\n{TWO}")).expect("change two.txt");
        let changed = Instant::now();
        let banner = wait_until(Duration::from_secs(5), || {
            capture(&session, &viewer)
                .contains("two.txt changed on disk. Press r to read it again.")
                .then_some(())
        });
        let seconds = changed.elapsed().as_secs_f64();
        log(start, &format!("{name} banner: shown {} after {seconds:.1}s", banner.is_some()));
        if banner.is_none() {
            fail(format!("{name} banner: no banner in 5 s:\n{}", capture(&session, &viewer)));
            continue;
        }
        keys(&rex, &viewer, "r");
        let after_r = wait_until(Duration::from_secs(5), || {
            let screen = capture(&session, &viewer);
            screen.contains("a new first line").then_some(screen)
        })
        .unwrap_or_default();
        // A wide pane shows the note box beside line 2 of the file. A narrow pane has no room for it,
        // so the notes list (`a`) gives the line number.
        let wide = after_r.lines().any(|line| line.chars().count() >= 100);
        let beside = after_r.lines().any(|line| line.contains("alpha line") && line.contains("comment"));
        let detached_of =
            |screen: &str| screen.lines().any(|line| line.trim_start_matches(['│', ' ']).starts_with('!'));
        keys(&rex, &viewer, "a");
        let screen = capture(&session, &viewer);
        let clear = !after_r.contains("changed on disk");
        let moved =
            after_r.contains("a new first line") && if wide { beside } else { screen.contains("l.2") };
        let detached = detached_of(&after_r) || detached_of(&screen);
        log(
            start,
            &format!("{name} reload: banner gone {clear}, note moved to l.2 {moved}, detached {detached}"),
        );
        if !clear || !moved || detached {
            fail(format!(
                "{name} reload: banner gone {clear}, note on l.2 {moved}, detached {detached}:\n\
                 after r:\n{after_r}\nafter a:\n{screen}"
            ));
        }
        keys(&rex, &viewer, "\x1b");
        // Close the viewer only while it is open: it asks about the unsent note, and `k` keeps the draft.
        if has_block(&rex, &viewer) {
            keys(&rex, &viewer, "q");
            keys(&rex, &viewer, "k");
        }
    }

    drop(cleanup);
    log(start, "cleanup done");
    assert!(failures.is_empty(), "live files failures:\n{}", failures.join("\n"));
}
