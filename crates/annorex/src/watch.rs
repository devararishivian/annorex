//! A watch for the open files. The agent can change a file while the user reads it. A thread looks
//! at the files every two seconds and reports what changed. Annorex never changes the files.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::files::{Stamp, stamp};

/// How often the thread looks at the files.
const EVERY: Duration = Duration::from_secs(2);

/// What happened to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// The size or the time of the last change is not the one that Annorex read.
    Modified,
    /// The file is gone, or it is not a file now.
    Deleted,
}

/// A change of one open file. The path is relative to the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub kind: ChangeKind,
}

/// The open files and the stamp of each at the last read. The viewer writes it. The thread reads it.
pub type Known = Arc<Mutex<Vec<(PathBuf, Stamp)>>>;

/// The files of `known` that are not as they were read, in the order of `known`.
pub fn changes(root: &Path, known: &[(PathBuf, Stamp)]) -> Vec<Change> {
    known
        .iter()
        .filter_map(|(path, old)| {
            let kind = match stamp(root, path) {
                None => ChangeKind::Deleted,
                Some(now) if now != *old => ChangeKind::Modified,
                Some(_) => return None,
            };
            Some(Change { path: path.clone(), kind })
        })
        .collect()
}

/// Start the thread. It reports each change again at every look, until the viewer reads the file
/// again. It ends when the viewer drops `known` or the receiver.
pub fn spawn(root: PathBuf, known: &Known) -> Receiver<Change> {
    let (sender, receiver) = mpsc::channel();
    let known = Arc::clone(known);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(EVERY);
            // The viewer holds the other reference. When it is gone, the viewer ended.
            if Arc::strong_count(&known) == 1 {
                return;
            }
            let list = known.lock().map(|list| list.clone()).unwrap_or_default();
            for change in changes(&root, &list) {
                if sender.send(change).is_err() {
                    return;
                }
            }
        }
    });
    receiver
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn known_of(root: &Path, names: &[&str]) -> Vec<(PathBuf, Stamp)> {
        names.iter().map(|name| (PathBuf::from(name), stamp(root, Path::new(name)).expect("stamp"))).collect()
    }

    #[test]
    fn watch_reports_modified_and_deleted_files() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join("a.txt"), "one").expect("a");
        std::fs::write(dir.path().join("b.txt"), "two").expect("b");
        std::fs::write(dir.path().join("c.txt"), "three").expect("c");
        let known = known_of(dir.path(), &["a.txt", "b.txt", "c.txt"]);
        assert_eq!(changes(dir.path(), &known), []);
        std::fs::write(dir.path().join("a.txt"), "one and more").expect("change a");
        std::fs::remove_file(dir.path().join("b.txt")).expect("remove b");
        assert_eq!(
            changes(dir.path(), &known),
            [
                Change { path: PathBuf::from("a.txt"), kind: ChangeKind::Modified },
                Change { path: PathBuf::from("b.txt"), kind: ChangeKind::Deleted },
            ]
        );
    }

    #[test]
    fn a_directory_in_place_of_a_file_counts_as_deleted() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join("a.txt"), "one").expect("a");
        let known = known_of(dir.path(), &["a.txt"]);
        std::fs::remove_file(dir.path().join("a.txt")).expect("remove");
        std::fs::create_dir(dir.path().join("a.txt")).expect("dir in place");
        assert_eq!(
            changes(dir.path(), &known),
            [Change { path: PathBuf::from("a.txt"), kind: ChangeKind::Deleted }]
        );
    }
}
