// Derived from plannotator-tui (https://github.com/plannotator/plannotator-tui) @ 34c238a, Copyright (c) 2026 Michael Ramos, MIT License. Changes for Annorex: the reader takes the sessions folder, not the Codex home; removed find_transcripts, find_transcripts_by_id, and thread_of; is_subagent takes the first line of a rollout; added sessions_for_cwd (the newest sessions of one folder that have a reply, up to a limit); parse_messages reads one file and keeps only the final answers and the prompts.
//! Codex CLI: `<sessions>/YYYY/MM/DD/rollout-<timestamp>-<thread>.jsonl`.
//!
//! One file per thread. The first line is a `session_meta` that carries the `cwd`. A resumed
//! thread appends to the same file, so the modification time ranks the sessions.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::{Message, Role};

/// The newest sessions of `cwd` that hold at least one reply, newest modification first, a
/// maximum of `limit`. Subagent rollouts do not count. The files are ranked by modification
/// time first. Only the first line of a file is read to match the folder, and a whole file is
/// read only until `limit` sessions with a reply are found.
pub fn sessions_for_cwd(sessions_dir: &Path, cwd: &Path, limit: usize) -> Vec<(PathBuf, SystemTime)> {
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let mut ranked: Vec<(PathBuf, SystemTime)> = all_rollouts(sessions_dir)
        .into_iter()
        .filter_map(|path| std::fs::metadata(&path).and_then(|data| data.modified()).ok().map(|t| (path, t)))
        .collect();
    ranked.sort_by(|(a, a_time), (b, b_time)| b_time.cmp(a_time).then_with(|| b.cmp(a)));
    ranked
        .into_iter()
        .filter(|(path, _)| {
            first_line(path).is_some_and(|line| {
                !is_subagent(&line) && meta_cwd(&line).is_some_and(|c| same_dir(&c, &cwd))
            })
        })
        .filter(|(path, _)| {
            std::fs::read_to_string(path).is_ok_and(|text| crate::has_reply(&parse_messages(&text, 1)))
        })
        .take(limit)
        .collect()
}

fn all_rollouts(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(all_rollouts(&path));
        } else if path.extension().is_some_and(|x| x == "jsonl")
            && path.file_stem().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rollout-"))
        {
            out.push(path);
        }
    }
    out
}

fn first_line(path: &Path) -> Option<String> {
    let mut line = String::new();
    let mut reader = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    (reader.read_line(&mut line).ok()? > 0).then_some(line)
}

/// The `cwd` of the `session_meta` on the first line.
fn meta_cwd(first_line: &str) -> Option<PathBuf> {
    let value: Value = serde_json::from_str(first_line.trim()).ok()?;
    (value.get("type").and_then(Value::as_str) == Some("session_meta"))
        .then(|| value.pointer("/payload/cwd").and_then(Value::as_str).map(PathBuf::from))
        .flatten()
}

/// `real_b` is the canonical form of the folder that is searched, made once by the caller.
fn same_dir(a: &Path, real_b: &Path) -> bool {
    a == real_b || std::fs::canonicalize(a).is_ok_and(|a| a == real_b)
}

/// Is the rollout with this first line a subagent rollout? Subagent rollouts (reviews,
/// guardians, spawned agents) record `source.subagent` in the session meta. A line that is
/// not JSON is not a subagent.
pub fn is_subagent(first_line: &str) -> bool {
    serde_json::from_str::<Value>(first_line.trim())
        .ok()
        .and_then(|v| v.pointer("/payload/source/subagent").cloned())
        .is_some()
}

/// Messages, newest first, through the `n`th newest reply. A reply is an assistant message
/// with the phase `final_answer` or with no phase: a `commentary` message is progress, not an
/// answer. A prompt is a completed `UserMessage` item. The context that Codex sends as a user
/// message (the environment) is not a prompt.
pub fn parse_messages(jsonl: &str, n: usize) -> Vec<Message> {
    let messages = jsonl
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter_map(|v| message_of(&v))
        .collect::<Vec<_>>();
    crate::through_replies(messages.into_iter().rev(), n)
}

fn message_of(v: &Value) -> Option<Message> {
    let at = v.get("timestamp").and_then(Value::as_str).map(str::to_owned);
    let kind = v.get("type").and_then(Value::as_str)?;
    let payload = v.get("payload")?;
    let (role, id, blocks, block_type) = match kind {
        "response_item" => {
            if payload.get("type").and_then(Value::as_str) != Some("message")
                || payload.get("role").and_then(Value::as_str) != Some("assistant")
                || !matches!(payload.get("phase").and_then(Value::as_str), None | Some("final_answer"))
            {
                return None;
            }
            (Role::Assistant, payload.get("id"), payload.get("content"), "output_text")
        }
        "event_msg" => {
            let item = payload.get("item")?;
            if payload.get("type").and_then(Value::as_str) != Some("item_completed")
                || item.get("type").and_then(Value::as_str) != Some("UserMessage")
            {
                return None;
            }
            (Role::Human, item.get("id"), item.get("content"), "text")
        }
        _ => return None,
    };
    let text: Vec<&str> = blocks?
        .as_array()?
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some(block_type))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    (!text.join("").trim().is_empty()).then(|| Message {
        id: id.and_then(Value::as_str).unwrap_or_default().to_owned(),
        role,
        text: text.join("\n\n"),
        at,
    })
}
