//! Live test of the installed module in the environment of a Rex action: a short `PATH`, no
//! `REX_BIN_DIR`, and `setup{}` with no `bin`.
//!
//! The Rex server runs actions with `PATH=/usr/bin:/bin:/usr/sbin:/sbin` and without
//! `REX_BIN_DIR`. This test runs the action with `rex do FILE --action` and the same environment.
//! It opens a plain shell pane and makes no model call. It runs only with `ANNOREX_LIVE=1`:
//!
//! ```text
//! ANNOREX_LIVE=1 cargo test -p annorex --test live_install -- --ignored --nocapture
//! ```

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    reason = "a live test is one script that reports by printing and asserts by panicking"
)]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

use annorex::rex::find_binary;
use common::{Cleanup, live_enabled, log, rex_json};

const BIN: &str = env!("CARGO_BIN_EXE_annorex");
const SHORT_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
const NOT_AGENT: &str = " is not supported";

/// Install the module into `dir` and write an `init.lua` with `init_line`.
fn install(dir: &Path, init_line: &str) -> PathBuf {
    let output =
        Command::new(BIN).args(["rex", "install", "--no-init", "--dir"]).arg(dir).output().expect("install");
    assert!(output.status.success(), "install: {}", String::from_utf8_lossy(&output.stderr));
    let init = dir.join("init.lua");
    std::fs::write(&init, format!("{init_line}\n")).expect("init.lua");
    init
}

/// `rex do INIT --action annorex_last` with only `HOME`, the short `PATH`, `REX_SERVER`, and
/// `REX_CONFIG`. `REX_CONFIG` names `init`, so the module refresh uses the test folder and never
/// the real config.
fn run_action(rex: &Path, home: &Path, init: &Path, session: &str, block: &str) -> Output {
    let mut command = Command::new(rex);
    command
        .arg("do")
        .arg(init)
        .args(["--action", "annorex_last", "-s", session, "-b", block])
        .env_clear()
        .env("HOME", home)
        .env("PATH", SHORT_PATH)
        .env("REX_CONFIG", init);
    if let Some(server) = std::env::var_os("REX_SERVER") {
        command.env("REX_SERVER", server);
    }
    command.output().expect("rex do runs")
}

#[test]
#[ignore = "uses a live Rex server; run with ANNOREX_LIVE=1 and --ignored"]
fn installed_module_works_with_the_environment_of_a_rex_action() {
    if !live_enabled() {
        return;
    }
    let start = Instant::now();
    let rex = find_binary(|key| std::env::var_os(key)).expect("rex");
    let real_home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    let mut cleanup = Cleanup {
        session: None,
        agents: Vec::new(),
        roots: annorex_hosts::roots::Roots::from_env(),
        dirs: Vec::new(),
    };
    let work = tempfile::Builder::new().prefix("annorex-live-install-").tempdir().expect("temp dir");
    let work_text = work.path().to_string_lossy().into_owned();
    let created = rex_json(&["new", "annorex-live-install", "--json", "-c", &work_text]);
    let session = created["session_id"].as_str().expect("session id").to_owned();
    cleanup.session = Some(session.clone());
    let block = created["initial_windows"][0]["block_ids"][0].as_str().expect("block id").to_owned();
    log(start, &format!("session {session}, shell pane {block}"));
    let mut failures: Vec<String> = Vec::new();

    // 1. The installed path: both binaries are found, so annorex reaches the agent check.
    let config = work.path().join("config");
    let init = install(&config, "require(\"annorex\").setup{}");
    let output = run_action(&rex, &real_home, &init, &session, &block);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    log(start, &format!("installed path: exit {:?}, {}", output.status.code(), stderr.trim()));
    if output.status.success() || !stderr.contains(NOT_AGENT) {
        failures.push(format!("installed path: expected {NOT_AGENT:?}, got {stderr:?}"));
    }

    // 2. No binary in any place: the message names every place.
    let homebrew = ["/opt/homebrew/bin/annorex", "/usr/local/bin/annorex"];
    if homebrew.iter().any(|path| Path::new(path).exists()) {
        log(start, "skipped the not-found case: annorex is installed by Homebrew");
    } else {
        let module = config.join("annorex.lua");
        let text = std::fs::read_to_string(&module).expect("module");
        let line = format!("local INSTALLED_BIN = \"{BIN}\"\n");
        assert!(text.contains(&line), "the module has the installed path");
        // A directory with the name of the binary does not count as found.
        let dir_bin = work.path().join("annorex");
        std::fs::create_dir_all(&dir_bin).expect("directory");
        let dir_line = format!("local INSTALLED_BIN = \"{}\"\n", dir_bin.display());
        std::fs::write(&module, text.replace(&line, &dir_line)).expect("module");
        let empty_home = work.path().join("home");
        std::fs::create_dir_all(&empty_home).expect("home");
        let output = run_action(&rex, &empty_home, &init, &session, &block);
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        log(start, &format!("not found: exit {:?}, {}", output.status.code(), stderr.trim()));
        let expected = format!(
            "annorex was not found; tried: {dir}, {home}/.cargo/bin/annorex, /opt/homebrew/bin/annorex, /usr/local/bin/annorex, {home}/.local/bin/annorex",
            dir = dir_bin.display(),
            home = empty_home.display()
        );
        if output.status.success() || !stderr.contains(&expected) {
            failures.push(format!("not found: expected {expected:?}, got {stderr:?}"));
        }
    }

    // 3. A bin from setup{} that does not exist is an error, with no fallback.
    let config = work.path().join("config-bin");
    let init = install(&config, "require(\"annorex\").setup{ bin = \"/nonexistent/annorex\" }");
    let output = run_action(&rex, &real_home, &init, &session, &block);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    log(start, &format!("bad bin: exit {:?}, {}", output.status.code(), stderr.trim()));
    let expected = "the annorex binary /nonexistent/annorex from setup{} does not exist";
    if output.status.success() || !stderr.contains(expected) {
        failures.push(format!("bad bin: expected {expected:?}, got {stderr:?}"));
    }

    // 4. A module of an older version, and an open on a shell pane. The open ends with `NotAgent`,
    // and the action then goes on to ask which agent to open. `rex open` must not refresh the module
    // now (see `refreshes_module`): a reload would come while the action runs. The refresh follows a
    // successful `annorex rex open` of an agent (see `refresh_module`), but this test starts no
    // agent, so it checks the guard only.
    let config = work.path().join("config-old");
    let init = install(&config, "require(\"annorex\").setup{}");
    let module = config.join("annorex.lua");
    let current = std::fs::read_to_string(&module).expect("module");
    let version = format!("(annorex {})", env!("CARGO_PKG_VERSION"));
    let old = current.replacen(&version, "(annorex 0.0.1)", 1);
    assert_ne!(old, current, "the old module differs from the current one");
    std::fs::write(&module, &old).expect("old module");
    let output = run_action(&rex, &real_home, &init, &session, &block);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    log(start, &format!("old module: exit {:?}, {}", output.status.code(), stderr.trim()));
    let after = std::fs::read_to_string(&module).expect("module");
    if after != old {
        failures.push(format!(
            "old module: an open on a shell pane must not refresh the module, got {:?}",
            after.lines().next()
        ));
    }
    if !stderr.contains(NOT_AGENT) {
        failures.push(format!("old module: expected {NOT_AGENT:?}, got {stderr:?}"));
    }

    // 5. The install without `--no-init`: it adds the setup block to `init.lua` of `--dir`, checks
    // it, and reloads Rex. The reload acts on the config of the running server, which the owner
    // accepts in a live test run by hand.
    let config = work.path().join("config-init");
    let output = Command::new(BIN).args(["rex", "install", "--dir"]).arg(&config).output().expect("install");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    log(start, &format!("install with init: exit {:?}, {}", output.status.code(), stdout.trim()));
    let init = config.join("init.lua");
    let created = format!("Created {} with these lines:", init.display());
    for expected in [created.as_str(), "Checked init.lua with rex config check: no errors."] {
        if !stdout.contains(expected) {
            failures.push(format!("install with init: expected {expected:?}, got {stdout:?}"));
        }
    }
    let text = std::fs::read_to_string(&init).unwrap_or_default();
    for line in ["require(\"annorex\").setup{}", "rex.bind(\"cmd+shift+a\", \"annorex_last\")"] {
        if !text.contains(line) {
            failures.push(format!("install with init: init.lua lacks {line:?}: {text:?}"));
        }
    }

    cleanup.dirs.push(work);
    drop(cleanup);
    assert!(failures.is_empty(), "{failures:#?}");
}
