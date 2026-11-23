//! The setup lines in `init.lua`: `annorex rex install` adds them when `init.lua` does not load
//! Annorex yet.

use std::path::{Path, PathBuf};

use crate::install::Check;

/// The block that `install` adds at the end of `init.lua`. A new `init.lua` gets it without the
/// first empty line.
pub const SETUP_BLOCK: &str = "\n-- Added by `annorex rex install`. Remove these lines to remove Annorex.\n\
require(\"annorex\").setup{}\n\
rex.bind(\"cmd+shift+a\", \"annorex_last\")\n\
rex.bind(\"cmd+shift+f\", \"annorex_files\")\n";

/// The key of the last-reply action in the setup block.
pub const KEY_LAST: &str = "cmd+shift+a";
/// The key of the files action in the setup block.
pub const KEY_FILES: &str = "cmd+shift+f";

/// What `init.lua` holds, as far as the setup is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitState {
    /// There is no `init.lua`.
    Missing,
    /// A line calls `require` with `annorex`, also when the line is a comment.
    Loaded,
    /// No line loads Annorex. The flags tell if `init.lua` binds the keys of the setup block.
    NotLoaded { binds_a: bool, binds_f: bool },
}

/// What `apply` did to `init.lua`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// `init.lua` did not exist. It now holds the setup block.
    Created,
    /// The setup block is at the end of `init.lua`. `skipped` holds the keys that `init.lua`
    /// binds already, so their `rex.bind` lines are not in the block.
    Appended { skipped: Vec<&'static str> },
    /// `init.lua` loads Annorex already.
    Unchanged,
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// `rex config check` found errors in the new `init.lua`. The message is what Rex printed.
    #[error("rex config check found errors:\n{message}")]
    /// `state` is what `init.lua` held, so the caller can print the lines to add by hand.
    CheckFailed { message: String, state: InitState },
    #[error("cannot change init.lua: {0}")]
    Io(#[from] std::io::Error),
}

/// Read what `init.lua` holds. Rulings: a `require` of `annorex` in any form counts, also in a
/// comment, because the user can turn it off on purpose. A key counts as bound when a line that
/// is not a comment calls `rex.bind` with it, in any modifier order.
pub fn inspect(text: &str) -> InitState {
    let mut rest = text;
    while let Some((_, after)) = rest.split_once("require") {
        // `pcall(require, "annorex")` gives the name after a comma.
        let argument = after.trim_start();
        let argument = argument.strip_prefix(',').unwrap_or(argument);
        if leading_string(argument) == Some("annorex") {
            return InitState::Loaded;
        }
        rest = after;
    }
    let (mut binds_a, mut binds_f) = (false, false);
    for line in text.lines().filter(|line| !line.trim_start().starts_with("--")) {
        let mut rest = line;
        while let Some((_, after)) = rest.split_once("rex.bind") {
            if let Some(key) = leading_string(after) {
                binds_a |= same_key(key, KEY_LAST);
                binds_f |= same_key(key, KEY_FILES);
            }
            rest = after;
        }
    }
    InitState::NotLoaded { binds_a, binds_f }
}

/// The Lua string at the start of `text`, after an optional `(`: the argument of a call such as
/// `require("x")`, `require 'x'`, `require[[x]]`, or `rex.bind("x", ...)`.
fn leading_string(text: &str) -> Option<&str> {
    let text = text.trim_start();
    let text = text.strip_prefix('(').unwrap_or(text).trim_start();
    if let Some(long) = text.strip_prefix("[[") {
        return long.split_once("]]").map(|(body, _)| body);
    }
    let quote = text.chars().next().filter(|ch| matches!(ch, '"' | '\''))?;
    text.strip_prefix(quote)?.split_once(quote).map(|(body, _)| body)
}

/// True when two Rex keys are the same key. Case and the order of the modifiers do not matter,
/// and `command`, `super`, `meta`, and `win` are `cmd` (Rex "Keyboard Shortcuts").
fn same_key(a: &str, b: &str) -> bool {
    fn parts(key: &str) -> (Vec<String>, String) {
        let key = key.to_lowercase();
        let mut parts: Vec<String> = key.split('+').map(str::to_owned).collect();
        let last = parts.pop().unwrap_or_default();
        let mut modifiers: Vec<String> = parts
            .into_iter()
            .map(|modifier| match modifier.as_str() {
                "command" | "super" | "meta" | "win" => "cmd".to_owned(),
                "control" => "ctrl".to_owned(),
                "opt" | "option" => "alt".to_owned(),
                _ => modifier,
            })
            .collect();
        modifiers.sort();
        modifiers.dedup();
        (modifiers, last)
    }
    parts(a) == parts(b)
}

/// The setup block for `state`. A `rex.bind` line of a key that `init.lua` binds already is left
/// out. `new_file` leaves out the first empty line.
pub fn block(state: &InitState, new_file: bool) -> String {
    let (binds_a, binds_f) = match *state {
        InitState::NotLoaded { binds_a, binds_f } => (binds_a, binds_f),
        InitState::Missing | InitState::Loaded => (false, false),
    };
    let skip_a = format!("rex.bind(\"{KEY_LAST}\"");
    let skip_f = format!("rex.bind(\"{KEY_FILES}\"");
    let skipped =
        |line: &str| (binds_a && line.starts_with(&skip_a)) || (binds_f && line.starts_with(&skip_f));
    let mut text = String::new();
    for line in SETUP_BLOCK.lines().filter(|line| !skipped(line)) {
        text.push_str(line);
        text.push('\n');
    }
    if new_file { text.trim_start_matches('\n').to_owned() } else { text }
}

/// The name of the check file beside `init.lua`.
fn check_name() -> String {
    format!(".init.lua.annorex-check.{}", std::process::id())
}

/// Add the setup block to the `init.lua` at `path`, or create it.
///
/// The new text goes first to a check file beside `path`, because `require` loads the files beside
/// the path that Rex gets, also when `init.lua` is a symbolic link. `check` gets the path of that
/// file. On `Check::Passed` or `Check::Skipped`, the new text replaces the file atomically. A
/// symbolic link stays: the file that it points to gets the text. The mode of the file stays.
pub fn apply(path: &Path, check: impl FnOnce(&Path) -> Check) -> Result<Change, SetupError> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let is_link = path.symlink_metadata().is_ok_and(|meta| meta.file_type().is_symlink());
    let target: PathBuf = if is_link { std::fs::canonicalize(path)? } else { path.to_path_buf() };
    let (text, mode, state) = match std::fs::read_to_string(&target) {
        Ok(text) => {
            let mode = std::fs::metadata(&target)?.permissions().mode();
            let state = inspect(&text);
            (text, Some(mode), state)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (String::new(), None, InitState::Missing)
        }
        Err(error) => return Err(error.into()),
    };
    if state == InitState::Loaded {
        return Ok(Change::Unchanged);
    }
    let line_end = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut contents = text.clone();
    if !text.is_empty() && !text.ends_with('\n') {
        contents.push_str(line_end);
    }
    contents.push_str(&block(&state, text.is_empty()).replace('\n', line_end));

    let dir = path.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = check_name();
    let check_file = dir.join(&name);
    // A file of an earlier run that stopped has the same name only with the same process ID.
    let _ = std::fs::remove_file(&check_file);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&check_file)
        .and_then(|mut file| std::io::Write::write_all(&mut file, contents.as_bytes()));
    let result = written.map(|()| check(&check_file));
    let _ = std::fs::remove_file(&check_file);
    match result? {
        Check::Failed(message) => {
            return Err(SetupError::CheckFailed { message: message.replace(&name, "init.lua"), state });
        }
        Check::Passed | Check::Skipped | Check::NoScratch => {}
    }
    crate::install::write_atomic_with_mode(&target, &contents, mode)?;
    let skipped = match state {
        InitState::NotLoaded { binds_a, binds_f } => [(binds_a, KEY_LAST), (binds_f, KEY_FILES)]
            .into_iter()
            .filter(|(bound, _)| *bound)
            .map(|(_, key)| key)
            .collect(),
        InitState::Missing | InitState::Loaded => Vec::new(),
    };
    Ok(if state == InitState::Missing { Change::Created } else { Change::Appended { skipped } })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::cell::RefCell;
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    const LOADED: InitState = InitState::Loaded;
    const NONE: InitState = InitState::NotLoaded { binds_a: false, binds_f: false };

    #[test]
    fn inspect_reads_the_require_forms_and_the_binds() {
        let cases: &[(&str, InitState)] = &[
            ("", NONE),
            ("rex.bind(\"cmd+d\", \"pane.split\")\n", NONE),
            ("require(\"annorex\").setup{}\n", LOADED),
            ("require('annorex').setup{}\n", LOADED),
            ("require \"annorex\"\n", LOADED),
            ("local a = require( \"annorex\" )\na.setup{}\n", LOADED),
            ("-- require(\"annorex\").setup{}\n", LOADED),
            ("local ok, m = pcall(require, \"annorex\")\n", LOADED),
            ("pcall(require,'annorex')\n", LOADED),
            ("require[[annorex]].setup{}\n", LOADED),
            ("require(\"annorex_extra\")\n", NONE),
            ("print(\"require annorex\")\n", NONE),
            ("require(\"keys\")\n", NONE),
            ("rex.bind(\"shift+cmd+a\", \"x\")\n", InitState::NotLoaded { binds_a: true, binds_f: false }),
            ("rex.bind('Command+Shift+F', \"x\")\n", InitState::NotLoaded { binds_a: false, binds_f: true }),
            (
                "rex.bind( \"cmd+shift+a\", f)\nrex.bind(\"super+shift+f\", \"y\")\n",
                InitState::NotLoaded { binds_a: true, binds_f: true },
            ),
            ("-- rex.bind(\"cmd+shift+a\", \"x\")\n", NONE),
            ("rex.bind(\"ctrl+cmd+shift+a\", \"x\")\n", NONE),
            ("rex.bind(\"cmd+a\", \"x\")\n", NONE),
            ("rex.unbind(\"cmd+shift+a\")\n", NONE),
        ];
        for (text, expected) in cases {
            assert_eq!(inspect(text), *expected, "{text:?}");
        }
    }

    #[test]
    fn block_leaves_out_the_bound_keys_and_the_first_empty_line() {
        assert_eq!(block(&NONE, false), SETUP_BLOCK);
        assert_eq!(block(&InitState::Missing, true), SETUP_BLOCK.trim_start_matches('\n'));
        let without_f = block(&InitState::NotLoaded { binds_a: false, binds_f: true }, false);
        assert!(without_f.contains("cmd+shift+a"), "{without_f}");
        assert!(!without_f.contains("cmd+shift+f"), "{without_f}");
        let without_both = block(&InitState::NotLoaded { binds_a: true, binds_f: true }, false);
        assert!(!without_both.contains("rex.bind"), "{without_both}");
        assert!(without_both.contains("require(\"annorex\").setup{}"), "{without_both}");
    }

    fn passed(_: &Path) -> Check {
        Check::Passed
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("dir")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn apply_creates_a_missing_file_and_folder() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("rex/init.lua");
        assert_eq!(apply(&path, passed).expect("apply"), Change::Created);
        let text = std::fs::read_to_string(&path).expect("init");
        assert_eq!(text, SETUP_BLOCK.strip_prefix('\n').expect("first empty line"));
        assert_eq!(names(&dir.path().join("rex")), ["init.lua"], "no check file stays");
    }

    #[test]
    fn apply_appends_once() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("init.lua");
        std::fs::write(&path, "rex.bind(\"cmd+d\", \"pane.split\")\n").expect("init");
        assert_eq!(apply(&path, passed).expect("apply"), Change::Appended { skipped: vec![] });
        assert_eq!(apply(&path, passed).expect("apply"), Change::Unchanged);
        let text = std::fs::read_to_string(&path).expect("init");
        assert_eq!(text, format!("rex.bind(\"cmd+d\", \"pane.split\")\n{SETUP_BLOCK}"));
        assert_eq!(text.matches("require(\"annorex\")").count(), 1);
    }

    #[test]
    fn apply_without_final_newline_starts_on_a_new_line() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("init.lua");
        std::fs::write(&path, "rex.bind(\"cmd+d\", \"pane.split\")").expect("init");
        apply(&path, passed).expect("apply");
        let text = std::fs::read_to_string(&path).expect("init");
        assert_eq!(text, format!("rex.bind(\"cmd+d\", \"pane.split\")\n{SETUP_BLOCK}"));
    }

    #[test]
    fn apply_with_crlf_lines() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("init.lua");
        std::fs::write(&path, "-- mine\r\nrex.bind(\"cmd+d\", \"pane.split\")").expect("init");
        apply(&path, passed).expect("apply");
        let text = std::fs::read_to_string(&path).expect("init");
        let block = SETUP_BLOCK.replace('\n', "\r\n");
        assert_eq!(text, format!("-- mine\r\nrex.bind(\"cmd+d\", \"pane.split\")\r\n{block}"));
    }

    #[test]
    fn apply_writes_through_a_symbolic_link() {
        let dir = tempfile::tempdir().expect("temp dir");
        let dotfiles = dir.path().join("dotfiles");
        let config = dir.path().join("config");
        std::fs::create_dir_all(&dotfiles).expect("dotfiles");
        std::fs::create_dir_all(&config).expect("config");
        let real = dotfiles.join("init.lua");
        std::fs::write(&real, "-- mine\n").expect("real");
        let link = config.join("init.lua");
        std::os::unix::fs::symlink(&real, &link).expect("link");
        assert_eq!(apply(&link, passed).expect("apply"), Change::Appended { skipped: vec![] });
        assert!(link.symlink_metadata().expect("link").file_type().is_symlink(), "the link stays");
        assert_eq!(std::fs::read_link(&link).expect("link"), real);
        assert_eq!(std::fs::read_to_string(&real).expect("real"), format!("-- mine\n{SETUP_BLOCK}"));
        assert_eq!(names(&dotfiles), ["init.lua"]);
        assert_eq!(names(&config), ["init.lua"]);
    }

    #[test]
    fn apply_follows_a_relative_symbolic_link() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("dotfiles")).expect("dotfiles");
        std::fs::create_dir_all(dir.path().join("config")).expect("config");
        std::fs::write(dir.path().join("dotfiles/init.lua"), "-- mine\n").expect("real");
        let link = dir.path().join("config/init.lua");
        std::os::unix::fs::symlink("../dotfiles/init.lua", &link).expect("link");
        apply(&link, passed).expect("apply");
        assert!(link.symlink_metadata().expect("link").file_type().is_symlink());
        let real = std::fs::read_to_string(dir.path().join("dotfiles/init.lua")).expect("real");
        assert_eq!(real, format!("-- mine\n{SETUP_BLOCK}"));
    }

    #[test]
    fn apply_keeps_the_file_mode() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("init.lua");
        std::fs::write(&path, "-- mine\n").expect("init");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("mode");
        apply(&path, passed).expect("apply");
        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn apply_does_not_write_when_the_check_fails() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("init.lua");
        std::fs::write(&path, "-- mine\n").expect("init");
        let result = apply(&path, |check| {
            let name = check.file_name().expect("name").to_string_lossy().into_owned();
            Check::Failed(format!("error {name}:3: boom"))
        });
        match result {
            Err(SetupError::CheckFailed { message, .. }) => assert_eq!(message, "error init.lua:3: boom"),
            other => panic!("expected CheckFailed, got {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(&path).expect("init"), "-- mine\n");
        assert_eq!(names(dir.path()), ["init.lua"], "no check file stays");
    }

    #[test]
    fn apply_does_not_create_a_file_when_the_check_fails() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("init.lua");
        let result = apply(&path, |_| Check::Failed("error x".to_owned()));
        assert!(matches!(result, Err(SetupError::CheckFailed { .. })), "{result:?}");
        assert!(names(dir.path()).is_empty(), "{:?}", names(dir.path()));
    }

    #[test]
    fn apply_writes_when_the_check_is_skipped() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("init.lua");
        assert_eq!(apply(&path, |_| Check::Skipped).expect("apply"), Change::Created);
        assert!(path.exists());
    }

    #[test]
    fn apply_runs_the_check_in_the_folder_of_init_lua() {
        let dir = tempfile::tempdir().expect("temp dir");
        let dotfiles = dir.path().join("dotfiles");
        let config = dir.path().join("config");
        std::fs::create_dir_all(&dotfiles).expect("dotfiles");
        std::fs::create_dir_all(&config).expect("config");
        std::fs::write(dotfiles.join("init.lua"), "require(\"memex\")\n").expect("real");
        let link = config.join("init.lua");
        std::os::unix::fs::symlink(dotfiles.join("init.lua"), &link).expect("link");
        let seen = RefCell::new(None);
        apply(&link, |check| {
            *seen.borrow_mut() =
                Some((check.to_path_buf(), std::fs::read_to_string(check).expect("check file")));
            Check::Passed
        })
        .expect("apply");
        let (check, text) = seen.into_inner().expect("check called");
        // Rex loads `require`d files beside the path that it gets, so the check file is beside
        // the path of init.lua (here the link), where annorex.lua and the other modules are.
        assert_eq!(check.parent(), Some(config.as_path()));
        assert_ne!(check, link);
        assert_eq!(text, format!("require(\"memex\")\n{SETUP_BLOCK}"));
    }

    #[test]
    fn apply_skips_a_conflicting_bind() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("init.lua");
        std::fs::write(&path, "rex.bind(\"shift+cmd+f\", \"find\")\n").expect("init");
        let change = apply(&path, passed).expect("apply");
        assert_eq!(change, Change::Appended { skipped: vec![KEY_FILES] });
        let text = std::fs::read_to_string(&path).expect("init");
        assert!(!text.contains("\"cmd+shift+f\""), "{text}");
        assert!(text.contains("rex.bind(\"cmd+shift+a\", \"annorex_last\")"), "{text}");
    }

    #[test]
    fn apply_to_a_loaded_init_lua_does_not_run_the_check() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("init.lua");
        std::fs::write(&path, "-- require(\"annorex\").setup{}\n").expect("init");
        let change = apply(&path, |_| panic!("no check for an unchanged file")).expect("apply");
        assert_eq!(change, Change::Unchanged);
        assert_eq!(std::fs::read_to_string(&path).expect("init"), "-- require(\"annorex\").setup{}\n");
    }
}
