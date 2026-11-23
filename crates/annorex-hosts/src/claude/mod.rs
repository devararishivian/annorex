// Derived from plannotator-tui (https://github.com/plannotator/plannotator-tui) @ 83b0cd9, Copyright (c) 2026 Michael Ramos, MIT License. Changes for Annorex: `project_slug` follows the memex rule (see the next header line); no other logic change.
// Derived from memex (https://github.com/nicosuave/memex) @ 9c5395e, Copyright (c) 2026 Nico Ritschel, MIT License. Changes for Annorex: `project_slug` and `base36` are ported from `claude_project_dir` and `base36` in `src/sources/claude.rs`; `project_slug` takes a `Path` and returns a `String`. The tests are Annorex code.
//! Claude Code: `~/.claude/sessions/<pid>.json` names a running session, and
//! `~/.claude/projects/<slug>/<session>.jsonl` is its transcript.

pub mod changes;
mod entries;
mod ladder;
mod messages;

use std::path::{Path, PathBuf};

use crate::SessionMeta;

pub use ladder::find_transcript;
pub use messages::{parse_messages, parse_messages_file_order};

/// Find `<projects>/*/<id>.jsonl`, preferring the current cwd's project bucket.
pub fn find_transcript_by_id(
    projects_dir: &Path,
    cwd: &Path,
    session_id: &str,
) -> Result<Option<PathBuf>, crate::HostError> {
    let id = crate::validate_session_id(session_id)?;
    let preferred = ladder::project_dir(projects_dir, cwd);
    if let Some(path) = preferred.as_ref().map(|dir| dir.join(format!("{id}.jsonl")))
        && path.is_file()
    {
        return Ok(Some(path));
    }
    let mut project_dirs: Vec<PathBuf> = std::fs::read_dir(projects_dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).filter(|path| path.is_dir()).collect())
        .unwrap_or_default();
    project_dirs.sort();
    Ok(project_dirs
        .into_iter()
        .filter(|dir| preferred.as_ref() != Some(dir))
        .map(|dir| dir.join(format!("{id}.jsonl")))
        .find(|path| path.is_file()))
}

/// Parse one `sessions/<pid>.json`. Fields beyond the four we use are ignored.
pub fn parse_session_meta(json: &str) -> Option<SessionMeta> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(SessionMeta {
        pid: u32::try_from(value.get("pid")?.as_u64()?).ok()?,
        session_id: value.get("sessionId")?.as_str()?.to_owned(),
        cwd: PathBuf::from(value.get("cwd")?.as_str()?),
        started_at: value.get("startedAt").and_then(serde_json::Value::as_u64).unwrap_or(0),
    })
}

/// Longest project directory name Claude Code uses before it appends a hash.
const PROJECT_DIR_MAX: usize = 200;

/// Claude Code's project directory name for a cwd, as Claude Code 2.1.330 computes it: every
/// UTF-16 code unit that is not an ASCII letter or digit becomes `-` (so `/Users/me/repo` is
/// `-Users-me-repo`). A name longer than 200 is cut to 200 and followed by `-` and the base-36
/// absolute value of the Java-style 32-bit string hash of the path.
pub fn project_slug(cwd: &Path) -> String {
    let text = cwd.to_string_lossy();
    let units: Vec<u16> = text.encode_utf16().collect();
    let name: String = units
        .iter()
        .map(|&unit| match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii_alphanumeric() => char::from(byte),
            _ => '-',
        })
        .collect();
    if name.len() <= PROJECT_DIR_MAX {
        return name;
    }
    let hash = units.iter().fold(0i32, |hash, &unit| hash.wrapping_mul(31).wrapping_add(i32::from(unit)));
    let head: String = name.chars().take(PROJECT_DIR_MAX).collect();
    format!("{head}-{}", base36(hash.unsigned_abs()))
}

fn base36(mut value: u32) -> String {
    let mut digits = Vec::new();
    loop {
        digits.push(char::from_digit(value % 36, 36).unwrap_or('0'));
        value /= 36;
        if value == 0 {
            break;
        }
    }
    digits.reverse();
    digits.into_iter().collect()
}

/// `ps -eo pid=,ppid=` output → `(pid, ppid)` pairs. Tolerates padding and blank lines.
pub fn parse_ps(output: &str) -> Vec<(u32, u32)> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            Some((pid, parent))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::project_slug;
    use std::path::Path;

    #[test]
    fn slug_maps_each_utf16_unit() {
        // U+1F600 is one character but two UTF-16 units, so it gives two dashes.
        assert_eq!(project_slug(Path::new("/a/\u{1F600}")), "-a---");
    }

    #[test]
    fn slug_of_a_long_path_is_cut_with_a_hash() {
        // Names that Claude Code 2.1.330 used, as traced in memex.
        let exactly_200 = format!("/work/{}", "a".repeat(194));
        assert_eq!(project_slug(Path::new(&exactly_200)), format!("-work-{}", "a".repeat(194)));
        let cases = [
            (format!("/work/{}", "a".repeat(195)), format!("-work-{}-9suycy", "a".repeat(194))),
            (
                format!("/work/{}/{}", "a".repeat(120), "b".repeat(123)),
                format!("-work-{}-{}-kvy58e", "a".repeat(120), "b".repeat(73)),
            ),
            // The hash runs over UTF-16 code units, including both halves of U+1D11E.
            (
                format!("/work/{}/{}/\u{1d11e}\u{1d11e}", "x".repeat(100), "\u{e9}".repeat(100)),
                format!("-work-{}{}-5jt2sd", "x".repeat(100), "-".repeat(94)),
            ),
        ];
        for (cwd, expected) in cases {
            assert_eq!(project_slug(Path::new(&cwd)), expected, "{cwd:?}");
        }
    }

    #[test]
    fn slug_keeps_the_old_result_for_a_plain_path() {
        assert_eq!(project_slug(Path::new("/Users/me/repo")), "-Users-me-repo");
    }
}
