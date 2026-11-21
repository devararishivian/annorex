// Written for Annorex. The fixtures are rollouts that Codex CLI 0.171.0 wrote.
//! Codex rollouts: replies, prompts, and finding the sessions of one folder.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};

use annorex_hosts::codex::{is_subagent, parse_messages, sessions_for_cwd};
use annorex_hosts::{Host, Role};

const CWD: &str = "/Users/dev/work/project";
const TURN_ONE: &str = "rollout-2026-11-21T14-50-48-01a1fd16-21c9-77d1-98ae-ab0ebe08e2bd.jsonl";
const TURN_THREE: &str = "rollout-2026-11-21T14-50-49-01a1fd16-0000-7000-8000-00000000a002.jsonl";
const ABORTED: &str = "rollout-2026-11-21T14-51-52-01a1fd17-1aa1-7311-9f0a-43b636420fd5.jsonl";
const PARENT: &str = "rollout-2026-11-21T14-52-33-01a1fd17-ba9e-73d2-8408-4c22938d6afd.jsonl";
const SUBAGENT: &str = "rollout-2026-11-21T14-52-41-01a1fd17-dbf8-79e2-8199-70841e228882.jsonl";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/codex/sessions/2026/11/21"))
            .join(name),
    )
    .expect("fixture")
}

fn replies(text: &str, n: usize) -> Vec<String> {
    parse_messages(text, n).into_iter().filter(|m| m.role == Role::Assistant).map(|m| m.text).collect()
}

/// Write a rollout whose first line is a `session_meta` for `cwd`, with an optional reply.
fn write_rollout(dir: &Path, name: &str, cwd: &str, reply: bool, seconds: u64) -> PathBuf {
    let meta = serde_json::json!({
        "timestamp": "2026-11-21T07:50:53.473Z",
        "type": "session_meta",
        "payload": {"id": name, "cwd": cwd, "source": "cli"}
    });
    let mut text = format!("{meta}\n");
    if reply {
        text.push_str(&assistant_line(Some("final_answer"), "done"));
    }
    std::fs::create_dir_all(dir).expect("dir");
    let path = dir.join(format!("rollout-{name}.jsonl"));
    std::fs::write(&path, text).expect("rollout");
    touch(&path, seconds);
    path
}

fn assistant_line(phase: Option<&str>, text: &str) -> String {
    let mut payload = serde_json::json!({
        "type": "message", "id": "m1", "role": "assistant",
        "content": [{"type": "output_text", "text": text}]
    });
    if let Some(phase) = phase {
        payload["phase"] = phase.into();
    }
    format!(
        "{}\n",
        serde_json::json!({"timestamp": "2026-11-21T07:51:03.708Z", "type": "response_item", "payload": payload})
    )
}

fn touch(path: &Path, seconds: u64) {
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    std::fs::File::options().write(true).open(path).expect("open").set_modified(time).expect("mtime");
}

fn names(found: &[(PathBuf, std::time::SystemTime)]) -> Vec<String> {
    found.iter().map(|(p, _)| p.file_name().expect("name").to_string_lossy().into_owned()).collect()
}

#[test]
fn final_answer_only() {
    let found = replies(&fixture(TURN_ONE), 10);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0], "The requested fixture changes are complete.");
    assert!(found[1].starts_with("## Changes"));
    assert!(found.iter().all(|t| !t.starts_with("I’ll add the notes file")));
}

#[test]
fn turn_in_progress_shows_the_last_final_answer() {
    let found = replies(&fixture(TURN_THREE), 1);
    assert_eq!(found, ["The requested fixture changes are complete."]);
}

#[test]
fn message_without_phase_is_a_reply() {
    let found = parse_messages(&assistant_line(None, "plain"), 5);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].role, Role::Assistant);
    assert_eq!(found[0].text, "plain");
    assert_eq!(found[0].id, "m1");
    assert_eq!(found[0].at.as_deref(), Some("2026-11-21T07:51:03.708Z"));
    assert_eq!(parse_messages(&assistant_line(Some("commentary"), "x"), 5).len(), 0);
}

#[test]
fn prompts_are_user_messages_only() {
    let messages = parse_messages(&fixture(TURN_ONE), 10);
    let prompts: Vec<_> = messages.iter().filter(|m| m.role == Role::Human).collect();
    assert_eq!(prompts.len(), 2);
    assert!(prompts.iter().all(|m| !m.text.contains("<environment_context>")));
    assert!(prompts.iter().any(|m| m.text.starts_with("Fixture turn 1.")));
}

#[test]
fn sessions_for_cwd_filters_by_folder() {
    let root = tempfile::tempdir().expect("temp");
    let day = root.path().join("2026/11/21");
    write_rollout(&day, "a", CWD, true, 1_000);
    write_rollout(&day, "b", "/Users/dev/other", true, 5_000);
    write_rollout(&root.path().join("2026/11/20"), "c", CWD, true, 3_000);
    let found = sessions_for_cwd(root.path(), Path::new(CWD), usize::MAX);
    assert_eq!(names(&found), ["rollout-c.jsonl", "rollout-a.jsonl"]);
    assert_eq!(found[0].1, std::time::UNIX_EPOCH + std::time::Duration::from_mins(50));
}

#[test]
fn sessions_for_cwd_matches_through_a_symbolic_link() {
    let root = tempfile::tempdir().expect("temp");
    let real = root.path().join("real");
    std::fs::create_dir(&real).expect("real");
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("link");
    let sessions = root.path().join("sessions");
    write_rollout(&sessions, "a", &real.to_string_lossy(), true, 1_000);
    assert_eq!(sessions_for_cwd(&sessions, &link, 5).len(), 1);
    assert_eq!(sessions_for_cwd(&sessions, &real, 5).len(), 1);
}

#[test]
fn sessions_for_cwd_skips_subagents_and_sessions_without_a_reply() {
    let root = tempfile::tempdir().expect("temp");
    let day = root.path().join("2026/11/21");
    std::fs::create_dir_all(&day).expect("day");
    for name in [TURN_ONE, ABORTED, PARENT, SUBAGENT] {
        std::fs::write(day.join(name), fixture(name)).expect("copy");
    }
    let found = sessions_for_cwd(root.path(), Path::new(CWD), usize::MAX);
    let mut found = names(&found);
    found.sort();
    assert_eq!(found, [TURN_ONE, PARENT], "no aborted turn and no subagent");
    let first_line = |name: &str| fixture(name).lines().next().expect("line").to_owned();
    assert!(is_subagent(&first_line(SUBAGENT)));
    assert!(!is_subagent(&first_line(PARENT)));
    assert!(!is_subagent("not json"));
}

#[test]
fn sessions_for_cwd_does_not_follow_a_folder_link_loop() {
    let root = tempfile::tempdir().expect("temp");
    let day = root.path().join("2026/11/21");
    write_rollout(&day, "a", CWD, true, 1_000);
    std::os::unix::fs::symlink(root.path(), day.join("loop")).expect("loop");
    let found = sessions_for_cwd(root.path(), Path::new(CWD), usize::MAX);
    assert_eq!(names(&found), ["rollout-a.jsonl"], "the link is not walked, so no copy of the file");
}

#[test]
fn sessions_for_cwd_stops_at_the_limit() {
    let root = tempfile::tempdir().expect("temp");
    for n in 0..5_u64 {
        write_rollout(root.path(), &format!("s{n}"), CWD, true, 1_000 + n);
    }
    let found = sessions_for_cwd(root.path(), Path::new(CWD), 2);
    assert_eq!(names(&found), ["rollout-s4.jsonl", "rollout-s3.jsonl"]);
}

#[test]
fn the_codex_host_label_is_codex() {
    assert_eq!(Host::Codex.label(), "codex");
}
