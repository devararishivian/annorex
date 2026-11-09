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
    assert!(out.contains("rex config reload"), "{out}");
    // The test binary is not in a PATH directory, so the bin option is shown too.
    assert!(out.contains("setup{ bin = \""), "{out}");
    assert!(dir.path().join("annorex.lua").is_file());
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
fn rex_open_rejects_unknown_placement() {
    let env = [("REX_SESSION", "session:x")];
    let output = annorex(&["rex", "open", "last", "--target", "block:b1", "--placement", "left"], &env, "");
    assert_eq!(output.status.code(), Some(64));
}
