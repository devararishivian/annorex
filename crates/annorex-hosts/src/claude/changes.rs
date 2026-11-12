//! The files that Claude Code changed: `Edit`, `Write`, `MultiEdit`, and `NotebookEdit` calls
//! whose result is not an error.

use serde_json::Value;

use crate::changes::{Changed, Event, collect};

const TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
/// Path keys in the order of use. Some `Write` calls use `path`; `NotebookEdit` uses `notebook_path`.
const PATH_KEYS: [&str; 3] = ["file_path", "path", "notebook_path"];

/// The files changed in the last `turns` turns of the human, newest first.
pub fn parse_changed(jsonl: &str, turns: usize) -> Vec<Changed> {
    collect(
        jsonl.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).flat_map(|v| events(&v)),
        turns,
    )
}

fn events(line: &Value) -> Vec<Event> {
    let blocks = line.pointer("/message/content");
    match (line.get("type").and_then(Value::as_str), blocks) {
        (Some("assistant"), Some(Value::Array(blocks))) => blocks.iter().filter_map(call).collect(),
        (Some("user"), Some(Value::Array(blocks))) => {
            let results: Vec<Event> = blocks.iter().filter_map(result).collect();
            if !results.is_empty() {
                results
            } else if is_meta(line) {
                Vec::new()
            } else {
                vec![Event::Prompt]
            }
        }
        (Some("user"), Some(Value::String(_))) if !is_meta(line) => vec![Event::Prompt],
        _ => Vec::new(),
    }
}

fn is_meta(line: &Value) -> bool {
    line.get("isMeta").and_then(Value::as_bool) == Some(true)
}

fn call(block: &Value) -> Option<Event> {
    if block.get("type")?.as_str()? != "tool_use" || !TOOLS.contains(&block.get("name")?.as_str()?) {
        return None;
    }
    let input = block.get("input")?;
    let path = PATH_KEYS.iter().find_map(|key| input.get(key)?.as_str())?;
    Some(Event::Call { id: block.get("id")?.as_str()?.to_owned(), path: path.to_owned() })
}

fn result(block: &Value) -> Option<Event> {
    if block.get("type")?.as_str()? != "tool_result" {
        return None;
    }
    let ok = block.get("is_error").and_then(Value::as_bool) != Some(true);
    Some(Event::Result { id: block.get("tool_use_id")?.as_str()?.to_owned(), ok })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(text: &str) -> String {
        format!(r#"{{"type":"user","message":{{"role":"user","content":"{text}"}}}}"#)
    }
    fn tool(id: &str, name: &str, key: &str, path: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"{id}","name":"{name}","input":{{"{key}":"{path}"}}}}]}}}}"#
        )
    }
    fn res(id: &str, error: bool) -> String {
        let flag = if error { r#","is_error":true"# } else { "" };
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"{id}","content":"x"{flag}}}]}}}}"#
        )
    }
    fn run(lines: &[String], turns: usize) -> Vec<String> {
        parse_changed(&lines.join("\n"), turns).into_iter().map(|c| c.path).collect()
    }

    #[test]
    fn counts_edit_and_write_with_a_good_result() {
        let lines = [
            prompt("go"),
            tool("1", "Edit", "file_path", "/w/a.rs"),
            res("1", false),
            tool("2", "Write", "file_path", "/w/b.rs"),
            res("2", false),
        ];
        assert_eq!(run(&lines, 1), ["/w/b.rs", "/w/a.rs"]);
    }

    #[test]
    fn ignores_a_call_with_is_error() {
        let lines = [prompt("go"), tool("1", "Edit", "file_path", "/w/a.rs"), res("1", true)];
        assert_eq!(run(&lines, 1), Vec::<String>::new());
    }

    #[test]
    fn ignores_a_call_without_a_result() {
        assert_eq!(run(&[prompt("go"), tool("1", "Edit", "file_path", "/w/a.rs")], 1), Vec::<String>::new());
    }

    #[test]
    fn ignores_other_tools() {
        let lines = [
            prompt("go"),
            tool("1", "Read", "file_path", "/w/a.rs"),
            res("1", false),
            tool("2", "Bash", "command", "sed -i x y"),
            res("2", false),
        ];
        assert_eq!(run(&lines, 1), Vec::<String>::new());
    }

    #[test]
    fn accepts_path_and_notebook_path() {
        let lines = [
            prompt("go"),
            tool("1", "Write", "path", "/w/a.rs"),
            res("1", false),
            tool("2", "NotebookEdit", "notebook_path", "/w/n.ipynb"),
            res("2", false),
        ];
        assert_eq!(run(&lines, 1), ["/w/n.ipynb", "/w/a.rs"]);
    }

    #[test]
    fn limits_to_the_last_turns() {
        let lines = [
            prompt("one"),
            tool("1", "Edit", "file_path", "/w/old.rs"),
            res("1", false),
            prompt("two"),
            tool("2", "Edit", "file_path", "/w/new.rs"),
            res("2", false),
        ];
        assert_eq!(run(&lines, 1), ["/w/new.rs"]);
        assert_eq!(run(&lines, 2), ["/w/new.rs", "/w/old.rs"]);
    }

    #[test]
    fn a_tool_result_is_not_a_prompt() {
        let lines = [
            prompt("go"),
            tool("1", "Edit", "file_path", "/w/a.rs"),
            res("1", false),
            tool("2", "Edit", "file_path", "/w/b.rs"),
            res("2", false),
        ];
        assert_eq!(run(&lines, 1).len(), 2);
    }

    #[test]
    fn skips_an_invalid_line() {
        let lines =
            ["not json".to_owned(), prompt("go"), tool("1", "Edit", "file_path", "/w/a.rs"), res("1", false)];
        assert_eq!(run(&lines, 1), ["/w/a.rs"]);
    }
}
