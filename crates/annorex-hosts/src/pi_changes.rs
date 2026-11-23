//! The files that Pi changed: `edit` and `write` calls whose `toolResult` is not an error.

use serde_json::Value;

use crate::changes::{Changed, Event, collect};

const TOOLS: [&str; 2] = ["edit", "write"];

/// The files changed in the last `turns` turns of the human, newest first.
pub fn parse_changed(jsonl: &str, turns: usize) -> Vec<Changed> {
    collect(
        jsonl.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).flat_map(|v| events(&v)),
        turns,
    )
}

pub(crate) fn events(line: &Value) -> Vec<Event> {
    if line.get("type").and_then(Value::as_str) != Some("message") {
        return Vec::new();
    }
    let Some(message) = line.get("message") else { return Vec::new() };
    match message.get("role").and_then(Value::as_str) {
        Some("user") => vec![Event::Prompt],
        Some("assistant") => message
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| blocks.iter().filter_map(call).collect())
            .unwrap_or_default(),
        Some("toolResult") => message
            .get("toolCallId")
            .and_then(Value::as_str)
            .map(|id| Event::Result {
                id: id.to_owned(),
                ok: message.get("isError").and_then(Value::as_bool) != Some(true),
            })
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

fn call(block: &Value) -> Option<Event> {
    if block.get("type")?.as_str()? != "toolCall" || !TOOLS.contains(&block.get("name")?.as_str()?) {
        return None;
    }
    let path = block.get("arguments")?.get("path")?.as_str()?;
    Some(Event::Call { id: block.get("id")?.as_str()?.to_owned(), path: path.to_owned() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> String {
        r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"go"}]}}"#.to_owned()
    }
    fn tool(id: &str, name: &str, args: &str) -> String {
        format!(
            r#"{{"type":"message","message":{{"role":"assistant","content":[{{"type":"toolCall","id":"{id}","name":"{name}","arguments":{args}}}]}}}}"#
        )
    }
    fn res(id: &str, error: bool) -> String {
        format!(
            r#"{{"type":"message","message":{{"role":"toolResult","toolCallId":"{id}","toolName":"x","isError":{error},"content":[]}}}}"#
        )
    }
    fn run(lines: &[String], turns: usize) -> Vec<String> {
        parse_changed(&lines.join("\n"), turns).into_iter().map(|c| c.path).collect()
    }

    #[test]
    fn counts_edit_and_write_with_a_good_result() {
        let lines = [
            user(),
            tool("1", "edit", r#"{"path":"/w/a.rs","edits":[]}"#),
            res("1", false),
            tool("2", "write", r#"{"path":"/w/b.rs","content":"x"}"#),
            res("2", false),
        ];
        assert_eq!(run(&lines, 1), ["/w/b.rs", "/w/a.rs"]);
    }

    #[test]
    fn ignores_an_error_and_a_call_without_a_result() {
        let lines = [
            user(),
            tool("1", "edit", r#"{"path":"/w/a.rs"}"#),
            res("1", true),
            tool("2", "write", r#"{"path":"/w/b.rs"}"#),
        ];
        assert_eq!(run(&lines, 1), Vec::<String>::new());
    }

    #[test]
    fn ignores_bash_and_codemode() {
        let lines = [
            user(),
            tool("1", "bash", r#"{"command":"sed -i x y"}"#),
            res("1", false),
            tool("2", "codemode", r#"{"code":"fs.write()"}"#),
            res("2", false),
        ];
        assert_eq!(run(&lines, 1), Vec::<String>::new());
    }

    #[test]
    fn limits_to_the_last_turns() {
        let lines = [
            user(),
            tool("1", "edit", r#"{"path":"/w/old.rs"}"#),
            res("1", false),
            user(),
            tool("2", "edit", r#"{"path":"/w/new.rs"}"#),
            res("2", false),
        ];
        assert_eq!(run(&lines, 1), ["/w/new.rs"]);
        assert_eq!(run(&lines, 2), ["/w/new.rs", "/w/old.rs"]);
    }

    #[test]
    fn skips_an_invalid_line() {
        let lines = ["{".to_owned(), user(), tool("1", "write", r#"{"path":"/w/a.rs"}"#), res("1", false)];
        assert_eq!(run(&lines, 1), ["/w/a.rs"]);
    }
}
