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
    let mut statement = connection
        .prepare(
            "SELECT m.id, m.data, p.id, p.data FROM message m \
             JOIN part p ON p.message_id = m.id WHERE m.session_id = ?1 \
             ORDER BY m.time_created ASC, m.id ASC, p.time_created ASC, p.id ASC",
        )
        .map_err(|e| sql_error(WHAT, &e))?;
    let rows = statement
        .query_map(params![session_id], |row| {
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
        if is_user && last_user.as_deref() != Some(&message_id) {
            last_user = Some(message_id);
            events.push(Event::Prompt);
        }
        events.extend(part_events(&part_id, &part));
    }
    Ok(collect(events, turns))
}

/// The events of one `part.data` payload. Only a tool part with a file path yields calls.
pub(crate) fn part_events(part_id: &str, part: &Value) -> Vec<Event> {
    if part.get("type").and_then(Value::as_str) != Some("tool") {
        return Vec::new();
    }
    let Some(tool) = part.get("tool").and_then(Value::as_str) else { return Vec::new() };
    let state = part.get("state").unwrap_or(&Value::Null);
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
        paths.into_iter().map(|path| Event::Call { id: part_id.to_owned(), path }).collect();
    if !events.is_empty() {
        events.push(Event::Result { id: part_id.to_owned(), ok });
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

    #[test]
    fn a_rejected_session_id_is_an_error() {
        let (_d, p) = db(&[]);
        assert!(changed_for_session(&p, "../x", 1).is_err());
    }
}
