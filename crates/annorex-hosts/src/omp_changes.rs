//! The files that `OhMyPi` changed: `edit` and `write` calls whose `toolResult` is not an error.
//!
//! The default `edit` mode of `OhMyPi` (`hashline`) has no `path` in its call: the call holds one
//! `input` text. The path is in the `details` of the result. Shapes follow can1357/oh-my-pi @ v19.1.1:
//! `packages/coding-agent/src/edit/index.ts` (`aggregateDetails`: `details.path` for one file,
//! `details.perFileResults[].path` for more) and `packages/coding-agent/src/tools/write.ts`
//! (`details.resolvedPath`). A call with a `path` argument counts as it does for Pi.

use serde_json::Value;

use crate::changes::{Changed, Event, collect};
use crate::pi_changes;

const TOOLS: [&str; 2] = ["edit", "write"];

/// The files changed in the last `turns` turns of the human, newest first.
pub fn parse_changed(jsonl: &str, turns: usize) -> Vec<Changed> {
    collect(
        jsonl.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).flat_map(|v| events(&v)),
        turns,
    )
}

fn events(line: &Value) -> Vec<Event> {
    let message =
        line.get("type").and_then(Value::as_str).filter(|t| *t == "message").and(line.get("message"));
    let Some(message) = message else { return Vec::new() };
    match message.get("role").and_then(Value::as_str) {
        // A message that the agent made itself is not a turn of the human.
        Some("user") if message.get("synthetic").and_then(Value::as_bool) == Some(true) => Vec::new(),
        Some("toolResult") => {
            let mut out = Vec::new();
            if let Some(id) = message.get("toolCallId").and_then(Value::as_str)
                && message.get("toolName").and_then(Value::as_str).is_some_and(|name| TOOLS.contains(&name))
            {
                out.extend(
                    result_paths(message).into_iter().map(|path| Event::Call { id: id.to_owned(), path }),
                );
            }
            out.extend(pi_changes::events(line));
            out
        }
        _ => pi_changes::events(line),
    }
}

/// The paths in the `details` of an `edit` or `write` result.
fn result_paths(message: &Value) -> Vec<String> {
    let Some(details) = message.get("details") else { return Vec::new() };
    let one = |key: &str| details.get(key).and_then(Value::as_str).map(str::to_owned);
    let each = details
        .get("perFileResults")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| file.get("path").and_then(Value::as_str).map(str::to_owned));
    one("path").into_iter().chain(each).chain(one("resolvedPath")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(extra: &str) -> String {
        format!(
            r#"{{"type":"message","message":{{"role":"user","content":[{{"type":"text","text":"go"}}]{extra}}}}}"#
        )
    }
    fn call(id: &str, name: &str, args: &str) -> String {
        format!(
            r#"{{"type":"message","message":{{"role":"assistant","content":[{{"type":"toolCall","id":"{id}","name":"{name}","arguments":{args}}}]}}}}"#
        )
    }
    fn result(id: &str, tool: &str, error: bool, details: &str) -> String {
        format!(
            r#"{{"type":"message","message":{{"role":"toolResult","toolCallId":"{id}","toolName":"{tool}","isError":{error},"content":[],"details":{details}}}}}"#
        )
    }
    fn run(lines: &[String], turns: usize) -> Vec<String> {
        parse_changed(&lines.join("\n"), turns).into_iter().map(|c| c.path).collect()
    }

    #[test]
    fn a_hashline_edit_has_its_path_only_in_the_result() {
        let lines = [
            user(""),
            call("1", "edit", r#"{"input":"@/w/a.rs\n..."}"#),
            result("1", "edit", false, r#"{"diff":"+x","path":"/w/a.rs"}"#),
        ];
        assert_eq!(run(&lines, 1), ["/w/a.rs"]);
    }

    #[test]
    fn an_edit_of_many_files_lists_each_file() {
        let lines = [
            user(""),
            call("1", "edit", r#"{"input":"..."}"#),
            result(
                "1",
                "edit",
                false,
                r#"{"diff":"","perFileResults":[{"path":"/w/a.rs"},{"path":"/w/b.rs"}]}"#,
            ),
        ];
        assert_eq!(run(&lines, 1), ["/w/b.rs", "/w/a.rs"], "newest first");
    }

    #[test]
    fn a_write_has_its_path_in_resolved_path() {
        let lines = [
            user(""),
            call("1", "write", r#"{"path":"[a.rs#AB]","content":"x"}"#),
            result("1", "write", false, r#"{"resolvedPath":"/w/a.rs"}"#),
        ];
        assert!(run(&lines, 1).contains(&"/w/a.rs".to_owned()));
    }

    #[test]
    fn an_error_result_counts_for_nothing() {
        let lines = [
            user(""),
            call("1", "edit", r#"{"input":"..."}"#),
            result("1", "edit", true, r#"{"diff":"","errorKind":"x"}"#),
            call("2", "write", r#"{"path":"/w/b.rs"}"#),
            result("2", "write", true, "{}"),
        ];
        assert_eq!(run(&lines, 1), Vec::<String>::new());
    }

    #[test]
    fn other_tools_and_synthetic_prompts_are_ignored() {
        let lines = [
            user(""),
            call("1", "edit", r#"{"input":"..."}"#),
            result("1", "edit", false, r#"{"path":"/w/old.rs"}"#),
            user(r#","synthetic":true"#),
            result("2", "bash", false, r#"{"path":"/w/not-a-file.rs"}"#),
        ];
        assert_eq!(run(&lines, 1), ["/w/old.rs"]);
    }

    #[test]
    fn a_call_with_a_path_argument_counts_as_in_pi() {
        let lines = [
            user(""),
            call("1", "edit", r#"{"path":"/w/a.rs","old_string":"a","new_string":"b"}"#),
            result("1", "edit", false, "{}"),
        ];
        assert_eq!(run(&lines, 1), ["/w/a.rs"]);
    }
}
