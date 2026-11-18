//! The files of the work directory: the tree, the path guard, and the loading of one file.
//!
//! Annorex only reads these files. It never writes, renames, or deletes them.

use std::cmp::Ordering;
use std::collections::VecDeque;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::payload::sanitize;

/// The most entries that the tree shows.
pub const TREE_LIMIT: usize = 20_000;
/// The largest file that Annorex opens, in bytes.
pub const FILE_LIMIT: u64 = 1024 * 1024;
/// A file with a NUL byte in this many first bytes counts as binary.
const BINARY_CHECK: usize = 8192;

/// A file or a directory of the tree. The path is relative to the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub is_dir: bool,
}

/// The result of a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    pub entries: Vec<Entry>,
    /// True when the root has more entries than the limit.
    pub truncated: bool,
}

/// What the loader knows about a file, to find a change later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// A file that the viewer can show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedFile {
    pub relative: PathBuf,
    /// The content after `payload::sanitize`: LF line ends and no control characters.
    pub text: String,
    pub stamp: Stamp,
}

/// Why a file cannot be opened. The messages say what Annorex tried.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("{} is outside the work directory, so Annorex does not open it", .0.display())]
    Outside(PathBuf),
    #[error("{} is a binary file, so Annorex does not show it", .0.display())]
    Binary(PathBuf),
    #[error("{} has {size} bytes; Annorex opens files up to {FILE_LIMIT} bytes", path.display())]
    TooLarge { path: PathBuf, size: u64 },
    #[error("{} does not exist", .0.display())]
    NotFound(PathBuf),
    #[error("{} is not a regular file", .0.display())]
    NotAFile(PathBuf),
    #[error("cannot read {}: {source}", path.display())]
    Io { path: PathBuf, source: std::io::Error },
}

// Adapted from plannotator-tui (MIT) @ e537abd, crates/plannotator-tui/src/tree.rs: the list of folders.
/// Folders of builds, caches, and version control. The tree never lists them and never enters them.
pub const SKIPPED_DIRS: [&str; 15] = [
    ".cache",
    ".direnv",
    ".git",
    ".hg",
    ".jj",
    ".svn",
    ".venv",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
    "out",
    "target",
    "vendor",
    "venv",
];

/// The tree under `root`, up to [`TREE_LIMIT`] entries. See [`scan_with`] for the rules.
pub fn scan(root: &Path, show_hidden: bool) -> Scan {
    scan_with(root, TREE_LIMIT, show_hidden)
}

/// The tree under `root`, up to `limit` entries. The walk goes breadth first, so a cut at the
/// limit drops the deepest entries first. It reads no ignore file, because the agent often writes
/// to ignored files. It never lists or enters a folder of [`SKIPPED_DIRS`]. Names that start with
/// a dot show only when `show_hidden` is true. It lists a link but never follows it.
pub fn scan_with(root: &Path, limit: usize, show_hidden: bool) -> Scan {
    let mut entries = Vec::new();
    let mut truncated = false;
    let mut queue = VecDeque::from([PathBuf::new()]);
    'walk: while let Some(folder) = queue.pop_front() {
        let Ok(items) = std::fs::read_dir(root.join(&folder)) else { continue };
        let mut children: Vec<Entry> = items
            .filter_map(Result::ok)
            .filter_map(|item| {
                let name = item.file_name();
                // The type of the entry itself: a link is not a folder.
                let is_dir = item.file_type().ok()?.is_dir();
                let hidden = name.as_encoded_bytes().starts_with(b".");
                let skipped = is_dir && SKIPPED_DIRS.iter().any(|skip| name == *skip);
                (!skipped && (show_hidden || !hidden)).then(|| Entry { path: folder.join(name), is_dir })
            })
            .collect();
        children.sort_by(compare);
        for child in children {
            if entries.len() == limit {
                truncated = true;
                break 'walk;
            }
            if child.is_dir {
                queue.push_back(child.path.clone());
            }
            entries.push(child);
        }
    }
    entries.sort_by(compare);
    Scan { entries, truncated }
}

/// Directories before files in each directory, then by name. A directory comes before its content.
fn compare(a: &Entry, b: &Entry) -> Ordering {
    let parts = |entry: &Entry| -> Vec<(bool, std::ffi::OsString)> {
        let names: Vec<_> = entry.path.components().map(Component::as_os_str).collect();
        let last = names.len().saturating_sub(1);
        names
            .iter()
            .enumerate()
            .map(|(index, name)| (index == last && !entry.is_dir, (*name).to_owned()))
            .collect()
    };
    parts(a).cmp(&parts(b))
}

/// The path of `input` relative to `root`, when the file is inside `root`. `input` can be
/// relative (to `root`) or absolute. Links are resolved first, so a link to the outside is refused.
pub fn resolve(root: &Path, input: &Path) -> Result<PathBuf, FileError> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| FileError::Io { path, source }
    };
    let root = root.canonicalize().map_err(io(root))?;
    let joined = if input.is_absolute() { input.to_path_buf() } else { root.join(input) };
    let canonical = match joined.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(FileError::NotFound(input.to_path_buf()));
        }
        Err(error) => return Err(FileError::Io { path: input.to_path_buf(), source: error }),
    };
    canonical.strip_prefix(&root).map(Path::to_path_buf).map_err(|_| FileError::Outside(input.to_path_buf()))
}

/// Read one file. `relative` must be inside `root` (see [`resolve`]); `load` checks again.
pub fn load(root: &Path, relative: &Path) -> Result<LoadedFile, FileError> {
    let relative = resolve(root, relative)?;
    let path = root.join(&relative);
    let io = |source| FileError::Io { path: relative.clone(), source };
    let metadata = std::fs::metadata(&path).map_err(io)?;
    if !metadata.is_file() {
        return Err(FileError::NotAFile(relative));
    }
    if metadata.len() > FILE_LIMIT {
        return Err(FileError::TooLarge { path: relative, size: metadata.len() });
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .and_then(|file| file.take(FILE_LIMIT + 1).read_to_end(&mut bytes))
        .map_err(io)?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(FileError::TooLarge { path: relative, size: bytes.len() as u64 });
    }
    if bytes.iter().take(BINARY_CHECK).any(|&byte| byte == 0) {
        return Err(FileError::Binary(relative));
    }
    let text = sanitize(&String::from_utf8_lossy(&bytes));
    Ok(LoadedFile {
        relative,
        text,
        stamp: Stamp { size: metadata.len(), modified: metadata.modified().ok() },
    })
}

/// The stamp of a file now. `None` when the file is gone or cannot be read.
pub fn stamp(root: &Path, relative: &Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(root.join(relative)).ok().filter(std::fs::Metadata::is_file)?;
    Some(Stamp { size: metadata.len(), modified: metadata.modified().ok() })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn write(root: &Path, name: &str, content: &str) {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(path, content).expect("write");
    }

    fn names(scan: &Scan) -> Vec<String> {
        scan.entries.iter().map(|entry| entry.path.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn scan_lists_ignored_files_and_skips_build_folders() {
        let dir = tempfile::tempdir().expect("dir");
        let root = dir.path();
        write(root, ".gitignore", "target/\n*.log\nplans/\n");
        write(root, "src/main.rs", "fn main() {}\n");
        write(root, "target/debug/x", "x");
        write(root, "node_modules/p/index.js", "x");
        write(root, "a.log", "log");
        write(root, "plans/x.md", "# plan\n");
        write(root, ".git/config", "[core]");
        write(root, "notes.md", "# notes\n");
        let scan = scan(root, false);
        let names = names(&scan);
        for wanted in ["src/main.rs", "notes.md", "a.log", "plans/x.md"] {
            assert!(names.contains(&wanted.to_owned()), "{wanted} in {names:?}");
        }
        assert!(
            !names
                .iter()
                .any(|n| n.starts_with("target") || n.starts_with("node_modules") || n.starts_with(".git")),
            "{names:?}"
        );
        assert!(!scan.truncated);
    }

    #[test]
    fn scan_lists_a_file_of_git_info_exclude() {
        let dir = tempfile::tempdir().expect("dir");
        let root = dir.path();
        write(root, ".git/info/exclude", "secret.md\n");
        write(root, "secret.md", "# secret\n");
        assert!(names(&scan(root, false)).contains(&"secret.md".to_owned()));
        assert!(names(&scan(root, true)).contains(&"secret.md".to_owned()));
    }

    #[test]
    fn scan_shows_dot_entries_only_when_asked() {
        let dir = tempfile::tempdir().expect("dir");
        let root = dir.path();
        for name in [".github/workflows/ci.yml", ".env", ".git/config", ".venv/x", "a.md"] {
            write(root, name, "x");
        }
        let hidden = names(&scan(root, false));
        assert_eq!(hidden, ["a.md"], "{hidden:?}");
        let shown = names(&scan(root, true));
        for wanted in [".github/workflows/ci.yml", ".env", ".github"] {
            assert!(shown.contains(&wanted.to_owned()), "{wanted} in {shown:?}");
        }
        assert!(
            !shown.iter().any(|n| n.starts_with(".git/") || n == ".git" || n.starts_with(".venv")),
            "{shown:?}"
        );
    }

    #[test]
    fn scan_sorts_directories_first_and_stops_at_the_limit() {
        let dir = tempfile::tempdir().expect("dir");
        let root = dir.path();
        for name in ["b.txt", "a.txt", "z/inner.txt", "y/inner.txt"] {
            write(root, name, "x");
        }
        assert_eq!(
            names(&scan(root, false)),
            ["y", "y/inner.txt", "z", "z/inner.txt", "a.txt", "b.txt"],
            "directories first, each before its content"
        );
        let limited = scan_with(root, 3, false);
        assert_eq!(limited.entries.len(), 3);
        assert!(limited.truncated);
        assert!(!scan_with(root, 6, false).truncated);
    }

    #[test]
    fn a_cut_at_the_limit_drops_the_deepest_entries_first() {
        let dir = tempfile::tempdir().expect("dir");
        let root = dir.path();
        for index in 0..20 {
            write(root, &format!("aaa/deep/f{index:02}.txt"), "x");
        }
        write(root, "src/main.rs", "fn main() {}\n");
        let limited = scan_with(root, 5, false);
        assert!(limited.truncated);
        let names = names(&limited);
        assert_eq!(names.len(), 5, "{names:?}");
        assert!(names.contains(&"src/main.rs".to_owned()), "{names:?}");
        assert!(names.contains(&"aaa/deep".to_owned()), "{names:?}");
    }

    #[test]
    fn scan_lists_a_file_with_a_skipped_name_and_does_not_enter_links() {
        let dir = tempfile::tempdir().expect("dir");
        let root = dir.path();
        write(root, "build", "make\n");
        write(root, "real/inner.txt", "x");
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).expect("link");
        let names = names(&scan(root, false));
        assert!(names.contains(&"build".to_owned()), "{names:?}");
        assert!(names.contains(&"link".to_owned()), "{names:?}");
        assert!(!names.iter().any(|name| name.starts_with("link/")), "{names:?}");
    }

    #[test]
    fn paths_outside_the_root_are_refused() {
        let outer = tempfile::tempdir().expect("dir");
        let root = outer.path().join("project");
        write(&root, "inside.txt", "in");
        write(outer.path(), "secret.txt", "secret");
        std::os::unix::fs::symlink(outer.path().join("secret.txt"), root.join("link-out.txt")).expect("link");
        std::os::unix::fs::symlink(root.join("inside.txt"), root.join("link-in.txt")).expect("link");
        assert_eq!(resolve(&root, Path::new("inside.txt")).expect("inside"), Path::new("inside.txt"));
        assert_eq!(resolve(&root, Path::new("link-in.txt")).expect("inside link"), Path::new("inside.txt"));
        for outside in ["../secret.txt", "link-out.txt"] {
            assert!(matches!(resolve(&root, Path::new(outside)), Err(FileError::Outside(_))), "{outside}");
        }
        let absolute = outer.path().join("secret.txt");
        assert!(matches!(resolve(&root, &absolute), Err(FileError::Outside(_))));
        assert!(matches!(load(&root, Path::new("link-out.txt")), Err(FileError::Outside(_))));
        assert!(matches!(resolve(&root, Path::new("missing.txt")), Err(FileError::NotFound(_))));
    }

    #[test]
    fn binary_and_large_files_are_refused() {
        let dir = tempfile::tempdir().expect("dir");
        let root = dir.path();
        std::fs::write(root.join("bin.dat"), b"abc\0def").expect("write");
        std::fs::write(root.join("big.txt"), vec![b'a'; usize::try_from(FILE_LIMIT).expect("limit") + 1])
            .expect("write");
        std::fs::write(root.join("edge.txt"), vec![b'a'; usize::try_from(FILE_LIMIT).expect("limit")])
            .expect("write");
        std::fs::create_dir(root.join("folder")).expect("dir");
        assert!(matches!(load(root, Path::new("bin.dat")), Err(FileError::Binary(_))));
        assert!(matches!(load(root, Path::new("big.txt")), Err(FileError::TooLarge { .. })));
        assert!(load(root, Path::new("edge.txt")).is_ok(), "exactly the limit is fine");
        assert!(matches!(load(root, Path::new("folder")), Err(FileError::NotAFile(_))));
        let message = load(root, Path::new("bin.dat")).expect_err("binary").to_string();
        assert!(message.contains("bin.dat") && message.contains("binary"), "{message}");
    }

    #[test]
    fn load_cleans_crlf_and_controls() {
        let dir = tempfile::tempdir().expect("dir");
        write(dir.path(), "w.txt", "one\r\ntwo\x1b[31m\r\n");
        let loaded = load(dir.path(), Path::new("w.txt")).expect("load");
        assert_eq!(loaded.text, "one\ntwo[31m\n");
        assert_eq!(loaded.relative, Path::new("w.txt"));
    }

    #[test]
    fn stamp_changes_when_the_file_changes() {
        let dir = tempfile::tempdir().expect("dir");
        write(dir.path(), "f.txt", "one");
        let first = stamp(dir.path(), Path::new("f.txt")).expect("stamp");
        write(dir.path(), "f.txt", "one two");
        let second = stamp(dir.path(), Path::new("f.txt")).expect("stamp");
        assert_ne!(first, second);
        std::fs::remove_file(dir.path().join("f.txt")).expect("remove");
        assert_eq!(stamp(dir.path(), Path::new("f.txt")), None);
    }
}
