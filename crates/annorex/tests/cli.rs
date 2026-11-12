//! The `annorex` binary: commands, usage errors, and exit codes.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::io::Write as _;
use std::process::{Command, Output, Stdio};

fn annorex(args: &[&str], env: &[(&str, &str)], stdin: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_annorex"));
    command.args(args).env_clear().stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("annorex starts");
    child.stdin.take().expect("stdin").write_all(stdin.as_bytes()).expect("stdin written");
    child.wait_with_output().expect("annorex ends")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn help_lists_probe_and_deliver() {
    let output = annorex(&["--help"], &[], "");
    assert_eq!(output.status.code(), Some(0));
    let help = text(&output.stdout);
    assert!(help.contains("probe"), "{help}");
    assert!(help.contains("deliver"), "{help}");
}

#[test]
fn missing_rex_binary_exits_4_and_names_paths() {
    let env = [("REX_BIN_DIR", "/nonexistent"), ("PATH", ""), ("REX_SESSION", "session:x")];
    let output = annorex(&["probe", "--target", "block:b1"], &env, "");
    assert_eq!(output.status.code(), Some(4));
    assert!(text(&output.stderr).contains("/nonexistent/rex"), "{}", text(&output.stderr));
}

#[test]
fn missing_session_exits_64() {
    let output = annorex(&["probe", "--target", "block:b1"], &[("PATH", "")], "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("session"), "{}", text(&output.stderr));
}

#[test]
fn deliver_with_empty_stdin_exits_64() {
    let env = [("REX_BIN_DIR", "/nonexistent"), ("PATH", ""), ("REX_SESSION", "session:x")];
    let output = annorex(&["deliver", "--target", "block:b1"], &env, "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("empty"), "{}", text(&output.stderr));
}

#[test]
fn unknown_argument_exits_64() {
    let output = annorex(&["probe", "--nope"], &[], "");
    assert_eq!(output.status.code(), Some(64));
}

#[test]
fn missing_target_exits_64() {
    let output = annorex(&["deliver"], &[("REX_SESSION", "session:x")], "note");
    assert_eq!(output.status.code(), Some(64));
}

#[test]
fn rex_install_prints_setup_lines() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().to_str().expect("utf-8 path");
    let output = annorex(&["rex", "install", "--dir", path], &[("PATH", "/usr/bin")], "");
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let out = text(&output.stdout);
    assert!(out.contains(&format!("{path}/annorex.lua")), "{out}");
    assert!(out.contains("require(\"annorex\").setup{}"), "{out}");
    assert!(out.contains("rex.bind(\"cmd+shift+a\", \"annorex_last\")"), "{out}");
    assert!(out.contains("rex.bind(\"cmd+shift+f\", \"annorex_files\")"), "{out}");
    assert!(out.contains("rex config reload"), "{out}");
    // The module starts the binary that installed it, so no bin option is needed.
    let bin = env!("CARGO_BIN_EXE_annorex");
    assert!(out.contains(&format!("Rex starts: {bin}")), "{out}");
    assert!(!out.contains("setup{ bin = \""), "{out}");
    let module = std::fs::read_to_string(dir.path().join("annorex.lua")).expect("module");
    assert!(module.contains(&format!("local INSTALLED_BIN = \"{bin}\"\n")), "{module}");
}

#[test]
fn rex_install_refuses_foreign_file_with_exit_3() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("annorex.lua"), "-- mine\n").expect("foreign");
    let path = dir.path().to_str().expect("utf-8 path");
    let output = annorex(&["rex", "install", "--dir", path], &[], "");
    assert_eq!(output.status.code(), Some(3));
    assert!(text(&output.stderr).contains("annorex.lua"), "{}", text(&output.stderr));
}

#[test]
fn rex_open_without_session_exits_64() {
    let output = annorex(&["rex", "open", "last", "--target", "block:b1"], &[("PATH", "")], "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("session"), "{}", text(&output.stderr));
}

#[test]
fn rex_open_has_no_placement_flag() {
    let env = [("REX_SESSION", "session:x")];
    let output = annorex(&["rex", "open", "last", "--target", "block:b1", "--placement", "right"], &env, "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("--placement"), "{}", text(&output.stderr));
}

#[test]
fn a_config_file_with_the_removed_placement_key_exits_64_and_names_the_key() {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::create_dir_all(dir.path().join("annorex")).expect("config dir");
    std::fs::write(dir.path().join("annorex/config.toml"), "placement = \"right\"\n").expect("config");
    let home = dir.path().to_string_lossy().into_owned();
    let env = [("XDG_CONFIG_HOME", home.as_str()), ("REX_SESSION", "session:x"), ("PATH", "")];
    let output = annorex(&["rex", "open", "last", "--target", "block:b1"], &env, "");
    assert_eq!(output.status.code(), Some(64), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("placement"), "{}", text(&output.stderr));
}

#[test]
fn last_without_session_exits_64() {
    let output = annorex(&["last", "--target", "block:b1"], &[("PATH", "")], "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("session"), "{}", text(&output.stderr));
}

#[test]
fn help_lists_last_and_rex() {
    let output = annorex(&["--help"], &[], "");
    assert_eq!(output.status.code(), Some(0));
    let help = text(&output.stdout);
    assert!(help.contains("last"), "{help}");
    assert!(help.contains("rex"), "{help}");
}

#[test]
fn bad_config_exits_64() {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::create_dir_all(dir.path().join("annorex")).expect("config dir");
    std::fs::write(dir.path().join("annorex/config.toml"), "colour = 1\n").expect("config");
    let home = dir.path().to_string_lossy().into_owned();
    let env = [("XDG_CONFIG_HOME", home.as_str()), ("REX_SESSION", "session:x"), ("PATH", "")];
    for args in
        [&["deliver", "--target", "block:b1"][..], &["rex", "open", "last", "--target", "block:b1"][..]]
    {
        let output = annorex(args, &env, "hello");
        assert_eq!(output.status.code(), Some(64), "{args:?}: {}", text(&output.stderr));
        assert!(text(&output.stderr).contains("colour"), "{}", text(&output.stderr));
    }
}

#[test]
fn files_command_parses_paths_and_target() {
    // The command line is valid, so the run goes on and fails at the Rex discovery (exit 4).
    let env = [("REX_BIN_DIR", "/nonexistent"), ("PATH", ""), ("REX_SESSION", "session:x")];
    let output = annorex(&["files", "--target", "block:b1", "README.md", "docs/a.md"], &env, "");
    assert_eq!(output.status.code(), Some(4), "{}", text(&output.stderr));
    let output = annorex(&["files", "--target", "block:b1", "--session", "session:x"], &env, "");
    assert_eq!(output.status.code(), Some(4), "{}", text(&output.stderr));
}

#[test]
fn files_without_target_or_session_exits_64() {
    let output = annorex(&["files"], &[("REX_SESSION", "session:x")], "");
    assert_eq!(output.status.code(), Some(64));
    let output = annorex(&["files", "--target", "block:b1"], &[("PATH", "")], "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("session"), "{}", text(&output.stderr));
}

#[test]
fn rex_open_files_without_session_exits_64() {
    let output = annorex(&["rex", "open", "files", "--target", "block:b1", "a.md"], &[("PATH", "")], "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("session"), "{}", text(&output.stderr));
}

#[test]
fn help_lists_files() {
    let help = text(&annorex(&["--help"], &[], "").stdout);
    assert!(help.contains("files"), "{help}");
}
