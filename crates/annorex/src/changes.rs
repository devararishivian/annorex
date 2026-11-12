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
    let candidates = replies::candidates(source, roots, process_table)?;
    candidates.into_iter().next().map(|candidate| candidate.session).ok_or_else(|| LoadError::NoTranscript {
        agent: source.agent,
        cwd: source.cwd.clone(),
        searched: String::new(),
    })
}

/// The paths of `raw` relative to `root`, in order, once each. A path outside `root`, a path
/// with `..` that leaves it, and a file that no longer exists are dropped.
pub fn normalize(root: &Path, raw: &[Changed]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for changed in raw {
        let Some(relative) = relative_inside(root, Path::new(&changed.path)) else { continue };
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

/// The look of the thread of a files viewer. It finds the session of `source` when it can, reads
/// the transcript only when the file changed, and reports a list only when it differs from the
/// last one. A failure gives `None`: the next look tries again.
pub fn looker(
    source: Source,
    roots: Roots,
    root: PathBuf,
    turns: usize,
) -> impl FnMut() -> Option<Vec<PathBuf>> + Send + 'static {
    let mut session: Option<SessionRef> = None;
    let mut stamp: Option<(u64, SystemTime)> = None;
    let mut last: Option<Vec<PathBuf>> = None;
    move || {
        if session.is_none() {
            let table = replies::process_table().unwrap_or_default();
            session = resolve(&source, &roots, &table).ok();
        }
        let current = session.as_ref()?;
        if let SessionRef::Claude(path) | SessionRef::Pi(path) = current {
            let meta = std::fs::metadata(path).ok()?;
            let now = (meta.len(), meta.modified().ok()?);
            if stamp == Some(now) && last.is_some() {
                return None;
            }
            stamp = Some(now);
        }
        let Ok(raw) = read(current, turns) else {
            session = None;
            return None;
        };
        let files = normalize(&root, &raw);
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
