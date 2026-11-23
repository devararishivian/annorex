//! The files that `OpenCode` changed: completed `edit`, `write`, and `apply_patch` parts.
//!
//! Reads the `message` and `part` tables of `opencode.db` (read-only). The path key is
//! `filePath` in v2.2.6 and `path` in newer sources, so both are accepted.

use std::path::Path;

use rusqlite::params;
use serde_json::Value;

use crate::HostError;
use crate::changes::{Changed, Event, collect};
use crate::sqlite::{open_read_only, sql_error};

const WHAT: &str = "OpenCode";
const PATH_KEYS: [&str; 2] = ["filePath", "path"];
/// Marker lines of an `apply_patch` text that name a file that exists after the patch.
const PATCH_MARKERS: [&str; 3] = ["*** Add File: ", "*** Update File: ", "*** Move to: "];

/// The files changed in the last `turns` turns of the human, newest first.
pub fn changed_for_session(db: &Path, session_id: &str, turns: usize) -> Result<Vec<Changed>, HostError> {
    crate::validate_session_id(session_id)?;
    let connection = open_read_only(db, WHAT)?;
    if let Some(events) = v2_events(&connection, session_id, turns)? {
        return Ok(collect(events, turns));
    }
    // Read only the messages from the Nth last prompt. Older turns cannot change the result.
    let (from_time, from_id) = v1_bound(&connection, session_id, turns).unwrap_or((i64::MIN, String::new()));
    let mut statement = connection
        .prepare(
            "SELECT m.id, m.data, p.id, p.data FROM message m \
             JOIN part p ON p.message_id = m.id WHERE m.session_id = ?1 \
             AND (m.time_created > ?2 OR (m.time_created = ?2 AND m.id >= ?3)) \
             ORDER BY m.time_created ASC, m.id ASC, p.time_created ASC, p.id ASC",
        )
        .map_err(|e| sql_error(WHAT, &e))?;
    let rows = statement
        .query_map(params![session_id, from_time, from_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| sql_error(WHAT, &e))?;
    let mut events = Vec::new();
    let mut last_user: Option<String> = None;
    for row in rows {
        let (message_id, message, part_id, part) = row.map_err(|e| sql_error(WHAT, &e))?;
        let part: Value = serde_json::from_str(&part).unwrap_or(Value::Null);
        let is_user = serde_json::from_str::<Value>(&message)
            .is_ok_and(|m| m.get("role").and_then(Value::as_str) == Some("user"));
        if is_user && is_typed_text(&part) && last_user.as_deref() != Some(&message_id) {
            last_user = Some(message_id);
            events.push(Event::Prompt);
        }
        events.extend(part_events(&part_id, &part));
    }
    Ok(collect(events, turns))
}

/// The time and the id of the message with the `turns`-th last prompt of the human. `None` when the
/// session has fewer prompts, or when the database cannot answer: then the caller reads everything.
fn v1_bound(connection: &rusqlite::Connection, session_id: &str, turns: usize) -> Option<(i64, String)> {
    let offset = i64::try_from(turns.max(1) - 1).ok()?;
    connection
        .query_row(
            "SELECT m.time_created, m.id FROM message m WHERE m.session_id = ?1 \
             AND json_valid(m.data) AND json_extract(m.data, '$.role') = 'user' \
             AND EXISTS (SELECT 1 FROM part p WHERE p.message_id = m.id AND json_valid(p.data) \
               AND json_extract(p.data, '$.type') = 'text' \
               AND coalesce(json_extract(p.data, '$.synthetic'), 0) != 1 \
               AND coalesce(json_extract(p.data, '$.ignored'), 0) != 1 \
               AND trim(coalesce(json_extract(p.data, '$.text'), '')) != '') \
             ORDER BY m.time_created DESC, m.id DESC LIMIT 1 OFFSET ?2",
            params![session_id, offset],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .ok()
}

/// The `seq` of the `turns`-th last prompt row in `session_message`. `i64::MIN` when there is none, or
/// when the database cannot answer: then the caller reads everything.
fn v2_bound(connection: &rusqlite::Connection, session_id: &str, turns: usize) -> i64 {
    let Ok(offset) = i64::try_from(turns.max(1) - 1) else { return i64::MIN };
    connection
        .query_row(
            "SELECT seq FROM session_message WHERE session_id = ?1 AND type = 'user' AND json_valid(data) \
             AND trim(coalesce(json_extract(data, '$.text'), '')) != '' \
             ORDER BY seq DESC LIMIT 1 OFFSET ?2",
            params![session_id, offset],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(i64::MIN)
}

/// A text part that the user typed: not empty, not `synthetic` (injected), not `ignored`.
fn is_typed_text(part: &Value) -> bool {
    let flagged = |key: &str| part.get(key).and_then(Value::as_bool) == Some(true);
    part.get("type").and_then(Value::as_str) == Some("text")
        && !flagged("synthetic")
        && !flagged("ignored")
        && part.get("text").and_then(Value::as_str).is_some_and(|text| !text.trim().is_empty())
}

/// The events of a session in the `OpenCode` 2 tables, or `None` when `session_message` has no
/// rows for it (the session is then in the `message` and `part` tables).
fn v2_events(
    connection: &rusqlite::Connection,
    session_id: &str,
    turns: usize,
) -> Result<Option<Vec<Event>>, HostError> {
    let has_table = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'session_message'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| sql_error(WHAT, &e))?;
    if has_table == 0 {
        return Ok(None);
    }
    let mut statement = connection
        .prepare(
            "SELECT id, type, data FROM session_message WHERE session_id = ?1 \
             AND type IN ('user', 'assistant') AND seq >= ?2 ORDER BY seq ASC",
        )
        .map_err(|e| sql_error(WHAT, &e))?;
    let from = v2_bound(connection, session_id, turns);
    let rows = statement
        .query_map(params![session_id, from], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .map_err(|e| sql_error(WHAT, &e))?;
    let has_parts = crate::opencode::has_table(connection, "session_part")?;
    let mut events = Vec::new();
    let mut any = false;
    for row in rows {
        let (id, kind, data) = row.map_err(|e| sql_error(WHAT, &e))?;
        any = true;
        let data: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
        if kind == "user" {
            if data.get("text").and_then(Value::as_str).is_some_and(|text| !text.trim().is_empty()) {
                events.push(Event::Prompt);
            }
            continue;
        }
        for (index, item) in
            crate::opencode::assistant_content(connection, has_parts, &id, &data)?.iter().enumerate()
        {
            if item.get("type").and_then(Value::as_str) != Some("tool") {
                continue;
            }
            let Some(tool) = item.get("name").and_then(Value::as_str) else { continue };
            let state = item.get("state").unwrap_or(&Value::Null);
            events.extend(tool_events(&format!("{id}:{index}"), tool, state));
        }
    }
    Ok(any.then_some(events))
}

/// The events of one `part.data` payload. Only a tool part with a file path yields calls.
pub(crate) fn part_events(part_id: &str, part: &Value) -> Vec<Event> {
    if part.get("type").and_then(Value::as_str) != Some("tool") {
        return Vec::new();
    }
    let Some(tool) = part.get("tool").and_then(Value::as_str) else { return Vec::new() };
    tool_events(part_id, tool, part.get("state").unwrap_or(&Value::Null))
}

/// The events of one tool call. `state` holds `status` and `input`.
fn tool_events(call_id: &str, tool: &str, state: &Value) -> Vec<Event> {
    let input = state.get("input").unwrap_or(&Value::Null);
    let paths: Vec<String> = match tool {
        "edit" | "write" => {
            PATH_KEYS.iter().find_map(|k| input.get(k)?.as_str()).map(str::to_owned).into_iter().collect()
        }
        "apply_patch" => input.get("patchText").and_then(Value::as_str).map(patch_paths).unwrap_or_default(),
        _ => return Vec::new(),
    };
    let ok = state.get("status").and_then(Value::as_str) == Some("completed");
    let mut events: Vec<Event> =
        paths.into_iter().map(|path| Event::Call { id: call_id.to_owned(), path }).collect();
    if !events.is_empty() {
        events.push(Event::Result { id: call_id.to_owned(), ok });
    }
    events
}

fn patch_paths(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| PATCH_MARKERS.iter().find_map(|marker| line.strip_prefix(marker)))
        .map(|path| path.trim().to_owned())
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use rusqlite::Connection;

    use super::*;

    fn part(tool: &str, status: &str, input: &str) -> String {
        format!(
            r#"{{"type":"tool","tool":"{tool}","callID":"c","state":{{"status":"{status}","input":{input}}}}}"#
        )
    }

    fn db(rows: &[(&str, &str, &str, &str)]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("opencode.db");
        let c = Connection::open(&path).expect("open");
        c.execute_batch(
            "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL);
             CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL);",
        )
        .expect("schema");
        for (i, (mid, role, pid, data)) in rows.iter().enumerate() {
            let t = i64::try_from(i).expect("small");
            c.execute(
                "INSERT OR IGNORE INTO message VALUES (?1,'s',?2,?2,?3)",
                params![mid, t, format!(r#"{{"role":"{role}"}}"#)],
            )
            .expect("message");
            c.execute("INSERT INTO part VALUES (?1,?2,'s',?3,?3,?4)", params![pid, mid, t, data])
                .expect("part");
        }
        (dir, path)
    }
    fn paths(db: &Path, turns: usize) -> Vec<String> {
        changed_for_session(db, "s", turns).expect("read").into_iter().map(|c| c.path).collect()
    }

    #[test]
    fn counts_a_completed_edit_and_write() {
        let (_d, p) = db(&[
            ("m1", "user", "p1", r#"{"type":"text","text":"go"}"#),
            ("m2", "assistant", "p2", &part("edit", "completed", r#"{"filePath":"/w/a.rs"}"#)),
            ("m2", "assistant", "p3", &part("write", "completed", r#"{"filePath":"/w/b.rs","content":"x"}"#)),
        ]);
        assert_eq!(paths(&p, 1), ["/w/b.rs", "/w/a.rs"]);
    }

    #[test]
    fn ignores_an_error_and_a_pending_state() {
        let (_d, p) = db(&[
            ("m1", "user", "p1", r#"{"type":"text","text":"go"}"#),
            ("m2", "assistant", "p2", &part("edit", "error", r#"{"filePath":"/w/a.rs"}"#)),
            ("m2", "assistant", "p3", &part("write", "pending", r#"{"filePath":"/w/b.rs"}"#)),
        ]);
        assert_eq!(paths(&p, 1), Vec::<String>::new());
    }

    #[test]
    fn accepts_file_path_and_path() {
        let (_d, p) = db(&[
            ("m1", "user", "p1", r#"{"type":"text","text":"go"}"#),
            ("m2", "assistant", "p2", &part("write", "completed", r#"{"path":"/w/a.rs"}"#)),
        ]);
        assert_eq!(paths(&p, 1), ["/w/a.rs"]);
    }

    #[test]
    fn reads_the_paths_of_an_apply_patch() {
        let patch = "*** Begin Patch\\n*** Add File: n.rs\\n+x\\n*** Update File: u.rs\\n*** Move to: m.rs\\n*** Delete File: d.rs\\n*** End Patch";
        let (_d, p) = db(&[
            ("m1", "user", "p1", r#"{"type":"text","text":"go"}"#),
            (
                "m2",
                "assistant",
                "p2",
                &part("apply_patch", "completed", &format!(r#"{{"patchText":"{patch}"}}"#)),
            ),
        ]);
        assert_eq!(paths(&p, 1), ["m.rs", "u.rs", "n.rs"]);
    }

    #[test]
    fn ignores_other_tools_and_text_parts() {
        let (_d, p) = db(&[
            ("m1", "user", "p1", r#"{"type":"text","text":"go"}"#),
            ("m2", "assistant", "p2", &part("bash", "completed", r#"{"command":"sed -i"}"#)),
            ("m2", "assistant", "p3", &part("read", "completed", r#"{"filePath":"/w/a.rs"}"#)),
        ]);
        assert_eq!(paths(&p, 1), Vec::<String>::new());
    }

    #[test]
    fn limits_to_the_last_turns() {
        let (_d, p) = db(&[
            ("m1", "user", "p1", r#"{"type":"text","text":"one"}"#),
            ("m2", "assistant", "p2", &part("edit", "completed", r#"{"filePath":"/w/old.rs"}"#)),
            ("m3", "user", "p3", r#"{"type":"text","text":"two"}"#),
            ("m4", "assistant", "p4", &part("edit", "completed", r#"{"filePath":"/w/new.rs"}"#)),
        ]);
        assert_eq!(paths(&p, 1), ["/w/new.rs"]);
        assert_eq!(paths(&p, 2), ["/w/new.rs", "/w/old.rs"]);
    }

    fn db_v2(rows: &[(&str, &str)]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("opencode.db");
        let c = Connection::open(&path).expect("open");
        c.execute_batch(
            "CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL, seq INTEGER NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL);",
        )
        .expect("schema");
        for (i, (kind, data)) in rows.iter().enumerate() {
            let n = i64::try_from(i).expect("small");
            c.execute(
                "INSERT INTO session_message VALUES (?1,'s',?2,?3,?3,?3,?4)",
                params![format!("m{i}"), kind, n, data],
            )
            .expect("row");
        }
        (dir, path)
    }
    fn item(name: &str, status: &str, input: &str) -> String {
        format!(
            r#"{{"content":[{{"type":"tool","id":"c","name":"{name}","state":{{"status":"{status}","input":{input}}}}}]}}"#
        )
    }

    #[test]
    fn v2_counts_completed_edit_and_write_and_limits_turns() {
        let (_d, p) = db_v2(&[
            ("user", r#"{"text":"one"}"#),
            ("assistant", &item("edit", "completed", r#"{"filePath":"/w/old.rs"}"#)),
            ("user", r#"{"text":"two"}"#),
            ("assistant", &item("write", "completed", r#"{"filePath":"/w/new.rs","content":"x"}"#)),
            ("assistant", &item("edit", "error", r#"{"filePath":"/w/bad.rs"}"#)),
            ("assistant", &item("bash", "completed", r#"{"command":"sed -i"}"#)),
        ]);
        assert_eq!(paths(&p, 1), ["/w/new.rs"]);
        assert_eq!(paths(&p, 2), ["/w/new.rs", "/w/old.rs"]);
    }

    #[test]
    fn v2_reads_an_apply_patch_and_the_path_key() {
        let patch = "*** Begin Patch\\n*** Add File: n.rs\\n+x\\n*** End Patch";
        let (_d, p) = db_v2(&[
            ("user", r#"{"text":"go"}"#),
            ("assistant", &item("apply_patch", "completed", &format!(r#"{{"patchText":"{patch}"}}"#))),
            ("assistant", &item("write", "completed", r#"{"path":"/w/p.rs"}"#)),
        ]);
        assert_eq!(paths(&p, 1), ["/w/p.rs", "n.rs"]);
    }

    #[test]
    fn v1_a_synthetic_user_message_does_not_start_a_turn() {
        let (_d, p) = db(&[
            ("m1", "user", "p1", r#"{"type":"text","text":"go"}"#),
            ("m2", "assistant", "p2", &part("edit", "completed", r#"{"filePath":"/w/a.rs"}"#)),
            ("m3", "user", "p3", r#"{"type":"text","text":"injected","synthetic":true}"#),
        ]);
        assert_eq!(paths(&p, 1), ["/w/a.rs"]);
    }

    #[test]
    fn v1_two_text_parts_of_one_prompt_are_one_turn() {
        let (_d, p) = db(&[
            ("m1", "user", "p1", r#"{"type":"text","text":"a"}"#),
            ("m1", "user", "p2", r#"{"type":"text","text":"b"}"#),
            ("m2", "assistant", "p3", &part("edit", "completed", r#"{"filePath":"/w/a.rs"}"#)),
        ]);
        assert_eq!(paths(&p, 1), ["/w/a.rs"]);
    }

    #[test]
    fn v2_an_empty_user_row_does_not_start_a_turn() {
        let (_d, p) = db_v2(&[
            ("user", r#"{"text":"go"}"#),
            ("assistant", &item("edit", "completed", r#"{"filePath":"/w/a.rs"}"#)),
            ("user", r#"{"text":"  "}"#),
        ]);
        assert_eq!(paths(&p, 1), ["/w/a.rs"]);
    }

    #[test]
    fn a_rejected_session_id_is_an_error() {
        let (_d, p) = db(&[]);
        assert!(changed_for_session(&p, "../x", 1).is_err());
    }
    #[test]
    fn v2_a_long_session_is_read_from_the_nth_last_prompt() {
        let mut rows: Vec<(String, String)> = Vec::new();
        for n in 0..200 {
            rows.push(("user".to_owned(), format!(r#"{{"text":"turn {n}"}}"#)));
            rows.push((
                "assistant".to_owned(),
                item("edit", "completed", &format!(r#"{{"filePath":"/w/f{n}.rs"}}"#)),
            ));
        }
        let borrowed: Vec<(&str, &str)> = rows.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let (_d, p) = db_v2(&borrowed);
        assert_eq!(paths(&p, 1), ["/w/f199.rs"]);
        assert_eq!(paths(&p, 3), ["/w/f199.rs", "/w/f198.rs", "/w/f197.rs"]);
        let connection = open_read_only(&p, WHAT).expect("open");
        // The last prompt is row 398 (0-based), the third last is row 394.
        assert_eq!(v2_bound(&connection, "s", 1), 398);
        assert_eq!(v2_bound(&connection, "s", 3), 394);
        assert_eq!(v2_bound(&connection, "s", 500), i64::MIN, "fewer prompts than turns: read all");
    }

    #[test]
    fn v2_a_session_without_a_prompt_reads_everything() {
        let (_d, p) = db_v2(&[("assistant", &item("edit", "completed", r#"{"filePath":"/w/a.rs"}"#))]);
        let connection = open_read_only(&p, WHAT).expect("open");
        assert_eq!(v2_bound(&connection, "s", 1), i64::MIN);
    }

    #[test]
    fn v1_a_long_session_is_read_from_the_nth_last_prompt() {
        let mut owned: Vec<(String, String, String, String)> = Vec::new();
        for n in 0..200 {
            owned.push((
                format!("m{n:03}a"),
                "user".into(),
                format!("p{n:03}a"),
                r#"{"type":"text","text":"go"}"#.into(),
            ));
            owned.push((
                format!("m{n:03}b"),
                "assistant".into(),
                format!("p{n:03}b"),
                part("edit", "completed", &format!(r#"{{"filePath":"/w/f{n}.rs"}}"#)),
            ));
        }
        let rows: Vec<(&str, &str, &str, &str)> =
            owned.iter().map(|(a, b, c, d)| (a.as_str(), b.as_str(), c.as_str(), d.as_str())).collect();
        let (_d, p) = db(&rows);
        assert_eq!(paths(&p, 1), ["/w/f199.rs"]);
        assert_eq!(paths(&p, 2), ["/w/f199.rs", "/w/f198.rs"]);
        let connection = open_read_only(&p, WHAT).expect("open");
        assert_eq!(v1_bound(&connection, "s", 1), Some((398, "m199a".to_owned())));
        assert_eq!(v1_bound(&connection, "s", 500), None);
    }

    #[test]
    fn v2_tool_parts_in_session_part_count_as_changes() {
        let (_d, p) = db_v2(&[("user", r#"{"text":"one"}"#), ("assistant", r#"{"content":[]}"#)]);
        let c = Connection::open(&p).expect("open");
        c.execute_batch(
            "CREATE TABLE session_part (message_id TEXT NOT NULL, session_id TEXT NOT NULL, ordinal INTEGER NOT NULL, type TEXT NOT NULL, call_id TEXT, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL);",
        )
        .expect("schema");
        let tool = r#"{"type":"tool","id":"c","name":"write","state":{"status":"completed","input":{"filePath":"/w/part.rs","content":"x"}}}"#;
        c.execute("INSERT INTO session_part VALUES ('m1','s',0,'tool','c',1,1,?1)", params![tool])
            .expect("part");
        assert_eq!(paths(&p, 1), ["/w/part.rs"]);
    }
}
