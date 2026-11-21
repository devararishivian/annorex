//! The files that Codex changed: the `FileChange` items of completed patches.
//!
//! A declined patch aborts the turn, so it writes no completed `FileChange`. A shell edit has
//! no path in the transcript, so it is not listed.

use serde_json::Value;

use crate::changes::{Changed, Event, collect};

/// The files changed in the last `turns` turns of the human, newest first.
pub fn parse_changed(jsonl: &str, turns: usize) -> Vec<Changed> {
    collect(
        jsonl.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).flat_map(|v| events(&v)),
        turns,
    )
}

fn events(line: &Value) -> Vec<Event> {
    if line.get("type").and_then(Value::as_str) != Some("event_msg") {
        return Vec::new();
    }
    let Some(payload) = line.get("payload") else { return Vec::new() };
    match payload.get("type").and_then(Value::as_str) {
        Some("task_started") => vec![Event::Prompt],
        Some("item_completed") => file_changes(payload.get("item")),
        _ => Vec::new(),
    }
}

/// One call and one result for each file of a `FileChange` item. The result is ok when the
/// item completed.
fn file_changes(item: Option<&Value>) -> Vec<Event> {
    let Some(item) = item else { return Vec::new() };
    if item.get("type").and_then(Value::as_str) != Some("FileChange") {
        return Vec::new();
    }
    let (Some(id), Some(changes)) =
        (item.get("id").and_then(Value::as_str), item.get("changes").and_then(Value::as_object))
    else {
        return Vec::new();
    };
    let ok = item.get("status").and_then(Value::as_str) == Some("completed");
    changes
        .iter()
        .flat_map(|(key, change)| {
            let path = change.get("move_path").and_then(Value::as_str).unwrap_or(key.as_str());
            let call = format!("{id}:{key}");
            [Event::Call { id: call.clone(), path: path.to_owned() }, Event::Result { id: call, ok }]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDITS: &str = include_str!(
        "../tests/fixtures/codex/sessions/2026/11/21/rollout-2026-11-21T14-50-48-01a1fd16-21c9-77d1-98ae-ab0ebe08e2bd.jsonl"
    );
    const DECLINED: &str = include_str!(
        "../tests/fixtures/codex/sessions/2026/11/21/rollout-2026-11-21T14-51-52-01a1fd17-1aa1-7311-9f0a-43b636420fd5.jsonl"
    );

    fn prompt() -> String {
        r#"{"type":"event_msg","payload":{"type":"task_started","turn_id":"t"}}"#.to_owned()
    }
    fn change(id: &str, status: &str, changes: &str) -> String {
        format!(
            r#"{{"type":"event_msg","payload":{{"type":"item_completed","item":{{"type":"FileChange","id":"{id}","changes":{changes},"status":"{status}"}}}}}}"#
        )
    }
    fn run(lines: &[String], turns: usize) -> Vec<String> {
        parse_changed(&lines.join("\n"), turns).into_iter().map(|c| c.path).collect()
    }

    #[test]
    fn lists_added_and_updated_files_newest_first() {
        // Turn 2 has no change, so the change of turn 1 is inside the last 2 turns.
        let got: Vec<String> = parse_changed(EDITS, 2).into_iter().map(|c| c.path).collect();
        assert_eq!(got, ["/Users/dev/work/project/docs/notes.md", "/Users/dev/work/project/README.md"]);
    }

    #[test]
    fn declined_change_is_not_listed() {
        assert_eq!(parse_changed(DECLINED, 1), Vec::<Changed>::new());
    }

    #[test]
    fn failed_change_is_not_listed() {
        let lines = [prompt(), change("1", "failed", r#"{"/w/a.md":{"type":"add"}}"#)];
        assert_eq!(run(&lines, 1), Vec::<String>::new());
    }

    #[test]
    fn move_gives_the_new_path() {
        let lines = [
            prompt(),
            change("1", "completed", r#"{"/w/old.md":{"type":"update","move_path":"/w/new.md"}}"#),
        ];
        assert_eq!(run(&lines, 1), ["/w/new.md"]);
    }

    #[test]
    fn limits_to_the_last_turns() {
        let lines = [
            prompt(),
            change("1", "completed", r#"{"/w/old.md":{"type":"add"}}"#),
            prompt(),
            change("2", "completed", r#"{"/w/new.md":{"type":"add"}}"#),
        ];
        assert_eq!(run(&lines, 1), ["/w/new.md"]);
        assert_eq!(run(&lines, 2), ["/w/new.md", "/w/old.md"]);
    }

    #[test]
    fn shell_edits_are_not_listed() {
        let shell = r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","id":"c","command":["sed","-i","x","src/main.rs"],"status":"completed"}}}"#;
        let lines = [prompt(), shell.to_owned()];
        assert_eq!(run(&lines, 1), Vec::<String>::new());
    }

    #[test]
    fn skips_an_invalid_line() {
        let lines = ["{".to_owned(), prompt(), change("1", "completed", r#"{"/w/a.md":{"type":"add"}}"#)];
        assert_eq!(run(&lines, 1), ["/w/a.md"]);
    }
}
