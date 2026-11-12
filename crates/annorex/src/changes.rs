//! The files that the agent changed in its last turns, as paths inside the work directory.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, SystemTime};

use annorex_hosts::changes::Changed;
use annorex_hosts::roots::Roots;
use annorex_hosts::{HostError, claude, opencode_changes, pi_changes};

use crate::replies::{self, LoadError, SessionRef, Source};

/// Read the files that the agent changed in the last `turns` turns of the human, as the agent
/// wrote the paths (newest first). A transcript that cannot be read gives an error.
pub fn read(session: &SessionRef, turns: usize) -> Result<Vec<Changed>, HostError> {
    match session {
        SessionRef::Claude(path) => {
            Ok(claude::changes::parse_changed(&std::fs::read_to_string(path)?, turns))
        }
        SessionRef::Pi(path) => Ok(pi_changes::parse_changed(&std::fs::read_to_string(path)?, turns)),
        SessionRef::OpenCode { db, found } => opencode_changes::changed_for_session(db, &found.id, turns),
    }
}

/// The session whose transcript holds the changes: the newest session of the agent.
pub fn resolve(
    source: &Source,
    roots: &Roots,
    process_table: &[(u32, u32)],
) -> Result<SessionRef, LoadError> {
    let found = replies::sessions(source, roots, process_table)?;
    found.into_iter().next().map(|(session, _)| session).ok_or_else(|| LoadError::NoTranscript {
        agent: source.agent,
        cwd: source.cwd.clone(),
        searched: String::new(),
    })
}

/// The paths of `raw` relative to `root`, in order, once each. A path outside `root`, a path
/// with `..` that leaves it, and a file that no longer exists are dropped.
pub fn normalize(root: &Path, raw: &[Changed]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    // The agent can write the real path of a directory that Rex reports through a link (`/var` and
    // `/private/var` on macOS).
    let real = root.canonicalize().ok();
    for changed in raw {
        let path = Path::new(&changed.path);
        let relative = relative_inside(root, path).or_else(|| relative_inside(real.as_deref()?, path));
        let Some(relative) = relative else { continue };
        if root.join(&relative).is_file() && !out.contains(&relative) {
            out.push(relative);
        }
    }
    out
}

/// `path` made relative to `root` without touching the disk, or `None` when it leaves `root`.
fn relative_inside(root: &Path, path: &Path) -> Option<PathBuf> {
    let relative = if path.is_absolute() { path.strip_prefix(root).ok()? } else { path };
    let mut out = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// How often the thread looks at the transcript.
pub const EVERY: Duration = Duration::from_secs(2);

/// Run `look` every `every` on a thread. `look` gives `Some(list)` when it has a list to report.
/// The thread ends when the receiver is gone or the returned token is dropped.
pub fn spawn_poll(
    every: Duration,
    mut look: impl FnMut() -> Option<Vec<PathBuf>> + Send + 'static,
) -> (Receiver<Vec<PathBuf>>, Arc<()>) {
    let (sender, receiver) = mpsc::channel();
    let token = Arc::new(());
    let held = Arc::clone(&token);
    std::thread::spawn(move || {
        loop {
            // The viewer holds the other reference. When it is gone, the viewer ended.
            if Arc::strong_count(&held) == 1 {
                return;
            }
            if let Some(list) = look()
                && sender.send(list).is_err()
            {
                return;
            }
            std::thread::sleep(every);
        }
    });
    (receiver, token)
}

/// The looks between two searches for the session. A new session (`/clear`, `/new`, a restart)
/// replaces the old one within this many looks.
const RESOLVE_EVERY: usize = 3;

/// The look of the thread of a files viewer for `source`. See [`looker_with`].
pub fn looker(
    source: Source,
    roots: Roots,
    root: PathBuf,
    turns: usize,
) -> impl FnMut() -> Option<Vec<PathBuf>> + Send + 'static {
    looker_with(
        move || {
            let table = replies::process_table().unwrap_or_default();
            resolve(&source, &roots, &table).ok()
        },
        root,
        turns,
    )
}

/// A look that finds the session with `find`, again every [`RESOLVE_EVERY`] looks, and reads the
/// transcript only when it changed. The first look and each look that has a new list give
/// `Some(list)`. A failure gives `None`: the next look tries again. A new session starts again.
pub fn looker_with(
    mut find: impl FnMut() -> Option<SessionRef> + Send + 'static,
    root: PathBuf,
    turns: usize,
) -> impl FnMut() -> Option<Vec<PathBuf>> + Send + 'static {
    let mut session: Option<SessionRef> = None;
    let mut stamp: Option<(u64, SystemTime)> = None;
    let mut raw: Option<Vec<Changed>> = None;
    let mut last: Option<Vec<PathBuf>> = None;
    let mut looks = 0usize;
    move || {
        if session.is_none() || looks.is_multiple_of(RESOLVE_EVERY) {
            match find() {
                Some(fresh) if session.as_ref().map(SessionRef::id) != Some(fresh.id()) => {
                    session = Some(fresh);
                    stamp = None;
                    raw = None;
                    last = None;
                }
                Some(_) | None => {}
            }
        }
        looks += 1;
        let current = session.as_ref()?;
        let mut unchanged = false;
        if let SessionRef::Claude(path) | SessionRef::Pi(path) = current {
            let meta = std::fs::metadata(path).ok()?;
            let now = (meta.len(), meta.modified().ok()?);
            unchanged = stamp == Some(now) && raw.is_some();
            stamp = Some(now);
        }
        if !unchanged {
            let Ok(fresh) = read(current, turns) else {
                session = None;
                return None;
            };
            raw = Some(fresh);
        }
        // A file that the agent changed and then removed leaves the list, also when the transcript is the same.
        let files = normalize(&root, raw.as_deref().unwrap_or_default());
        if last.as_ref() == Some(&files) {
            return None;
        }
        last = Some(files.clone());
        Some(files)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn changed(paths: &[&str]) -> Vec<Changed> {
        paths.iter().map(|p| Changed { path: (*p).to_owned() }).collect()
    }
    fn root_with(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("dir");
        for file in files {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, "x").expect("file");
        }
        dir
    }

    #[test]
    fn makes_an_absolute_path_relative() {
        let dir = root_with(&["src/a.rs"]);
        let abs = dir.path().join("src/a.rs");
        assert_eq!(
            normalize(dir.path(), &changed(&[abs.to_str().expect("utf8")])),
            [PathBuf::from("src/a.rs")]
        );
    }

    #[test]
    fn keeps_a_relative_path_and_the_order() {
        let dir = root_with(&["a.rs", "b.rs"]);
        assert_eq!(
            normalize(dir.path(), &changed(&["./b.rs", "a.rs", "b.rs"])),
            [PathBuf::from("b.rs"), PathBuf::from("a.rs")]
        );
    }

    #[test]
    fn accepts_the_real_path_of_a_root_that_is_a_link() {
        let real = root_with(&["a.rs"]);
        let parent = tempfile::tempdir().expect("dir");
        let link = parent.path().join("link");
        std::os::unix::fs::symlink(real.path(), &link).expect("link");
        let abs = real.path().canonicalize().expect("canonical").join("a.rs");
        assert_eq!(normalize(&link, &changed(&[abs.to_str().expect("utf8")])), [PathBuf::from("a.rs")]);
    }

    #[test]
    fn drops_a_path_outside_the_root() {
        let dir = root_with(&["a.rs"]);
        let other = root_with(&["o.rs"]);
        let outside = other.path().join("o.rs");
        assert_eq!(
            normalize(dir.path(), &changed(&[outside.to_str().expect("utf8")])),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn drops_dot_dot_that_leaves_the_root() {
        let dir = root_with(&["a.rs"]);
        assert_eq!(normalize(dir.path(), &changed(&["../a.rs", "x/../../a.rs"])), Vec::<PathBuf>::new());
        assert_eq!(normalize(dir.path(), &changed(&["x/../a.rs"])), [PathBuf::from("a.rs")]);
    }

    fn claude_line(kind: &str, body: &str) -> String {
        format!(r#"{{"type":"{kind}","message":{{"role":"{kind}","content":{body}}}}}"#)
    }
    fn edit_lines(id: &str, path: &str) -> String {
        format!(
            "{}\n{}\n",
            claude_line(
                "assistant",
                &format!(
                    r#"[{{"type":"tool_use","id":"{id}","name":"Edit","input":{{"file_path":"{path}"}}}}]"#
                )
            ),
            claude_line(
                "user",
                &format!(r#"[{{"type":"tool_result","tool_use_id":"{id}","content":"ok"}}]"#)
            ),
        )
    }

    #[test]
    fn the_looker_reports_a_new_list_only_and_drops_a_removed_file() {
        let root = root_with(&["a.rs", "b.rs"]);
        let log = tempfile::tempdir().expect("dir");
        let transcript = log.path().join("s1.jsonl");
        let mut text = format!("{}\n{}", claude_line("user", r#""go""#), edit_lines("1", "a.rs"));
        std::fs::write(&transcript, &text).expect("transcript");
        let session = SessionRef::Claude(transcript.clone());
        let mut look = looker_with(move || Some(session.clone()), root.path().to_path_buf(), 1);
        assert_eq!(look(), Some(vec![PathBuf::from("a.rs")]));
        assert_eq!(look(), None, "the same list is not sent again");
        text.push_str(&edit_lines("2", "b.rs"));
        std::fs::write(&transcript, &text).expect("append");
        assert_eq!(look(), Some(vec![PathBuf::from("b.rs"), PathBuf::from("a.rs")]));
        std::fs::remove_file(root.path().join("a.rs")).expect("remove");
        assert_eq!(look(), Some(vec![PathBuf::from("b.rs")]), "a removed file leaves the list");
    }

    #[test]
    fn the_looker_switches_to_a_new_session() {
        let root = root_with(&["a.rs", "b.rs"]);
        let log = tempfile::tempdir().expect("dir");
        for (name, file) in [("old.jsonl", "a.rs"), ("new.jsonl", "b.rs")] {
            let text = format!("{}\n{}", claude_line("user", r#""go""#), edit_lines("1", file));
            std::fs::write(log.path().join(name), text).expect("transcript");
        }
        let current = Arc::new(std::sync::Mutex::new(log.path().join("old.jsonl")));
        let chosen = Arc::clone(&current);
        let mut look = looker_with(
            move || Some(SessionRef::Claude(chosen.lock().expect("lock").clone())),
            root.path().to_path_buf(),
            1,
        );
        assert_eq!(look(), Some(vec![PathBuf::from("a.rs")]));
        *current.lock().expect("lock") = log.path().join("new.jsonl");
        let mut got = None;
        for _ in 0..RESOLVE_EVERY {
            got = got.or_else(&mut look);
        }
        assert_eq!(got, Some(vec![PathBuf::from("b.rs")]));
    }

    #[test]
    fn the_poll_thread_sends_what_look_returns_and_stops_with_the_token() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let (rx, token) = spawn_poll(Duration::from_millis(5), move || {
            let n = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (n == 1).then(|| vec![PathBuf::from("a.rs")])
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).expect("a list"), [PathBuf::from("a.rs")]);
        drop(token);
        std::thread::sleep(Duration::from_millis(50));
        let after = calls.load(std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), after, "the thread ended");
    }

    #[test]
    fn drops_a_file_that_is_gone() {
        let dir = root_with(&["a.rs"]);
        assert_eq!(normalize(dir.path(), &changed(&["gone.rs", "a.rs"])), [PathBuf::from("a.rs")]);
    }
}
