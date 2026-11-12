//! The files that the agent changed in its last turns, as paths inside the work directory.

use std::path::{Component, Path, PathBuf};

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
    fn drops_a_file_that_is_gone() {
        let dir = root_with(&["a.rs"]);
        assert_eq!(normalize(dir.path(), &changed(&["gone.rs", "a.rs"])), [PathBuf::from("a.rs")]);
    }
}
