// Derived from oh-my-pi (https://github.com/can1357/oh-my-pi) @ 31ffee5 (v19.1.1), Copyright (c) 2025 Mario Zechner, Copyright (c) 2025-2026 Can Bölük, Copyright (c) 2026 Stencil Labs, Inc., MIT License. Changes for Annorex: the folder-name rule of `getDefaultSessionDirName` written again in Rust; all three folder names are tried.

//! `OhMyPi` (`omp`): the Pi session tree under other folder names.
//!
//! The files and entries have the Pi shape, so [`parse_messages`] is the Pi reader. Two things
//! differ: the folder of a cwd (below) and the first line of a file, a 256-byte `title` record
//! without an `id` that the chain walk skips. Names follow can1357/oh-my-pi @ v19.1.1,
//! `packages/coding-agent/src/session/session-paths.ts` (`getDefaultSessionDirName`).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::pi;

pub use crate::pi::parse_messages;

/// The folder names that can hold the sessions of `cwd`, the likely one first: `-tmp-<rel>` under the
/// temp root, `-<rel>` under home, and `--<abs>--`. `OhMyPi` moves old names to the new one on start,
/// but a session folder that it did not start in can keep the old name. `home` and `temp_root` are
/// resolved with the file system when they exist, as `OhMyPi` does.
pub fn bucket_names(cwd: &Path, home: &Path, temp_root: &Path) -> Vec<String> {
    let cwd = real(cwd);
    let mut names = Vec::new();
    if let Some(relative) = relative_to(&cwd, &real(temp_root)) {
        names.push(relative_name("-tmp", &relative));
    }
    if let Some(relative) = relative_to(&cwd, &real(home)) {
        names.push(relative_name("-", &relative));
    }
    names.push(pi::encoded_dir(&cwd));
    names.dedup();
    names
}

/// The newest sessions of `cwd` that hold a reply, newest modification first, a maximum of `limit`.
/// The folders of all names in [`bucket_names`] count together.
pub fn sessions_for_cwd(
    sessions_dir: &Path,
    cwd: &Path,
    home: &Path,
    temp_root: &Path,
    limit: usize,
) -> Vec<(PathBuf, SystemTime)> {
    let dirs: Vec<PathBuf> =
        bucket_names(cwd, home, temp_root).into_iter().map(|name| sessions_dir.join(name)).collect();
    pi::ranked_sessions(&dirs, limit)
}

fn real(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The part of `path` below `root`: empty for `root` itself, `None` outside it.
fn relative_to(path: &Path, root: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    Some(rest.to_string_lossy().into_owned())
}

fn relative_name(prefix: &str, relative: &str) -> String {
    let encoded: String =
        relative.chars().map(|c| if matches!(c, '/' | '\\' | ':') { '-' } else { c }).collect();
    match (encoded.is_empty(), prefix.ends_with('-')) {
        (true, _) => prefix.to_owned(),
        (false, true) => format!("{prefix}{encoded}"),
        (false, false) => format!("{prefix}-{encoded}"),
    }
}
