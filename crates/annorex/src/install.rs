//! `annorex rex install`: write the Lua module beside the `init.lua` that Rex reads.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The Lua module. The binary carries it, so the module and the binary have the same version.
pub const MODULE_SOURCE: &str = include_str!("../lua/annorex.lua");

/// The first line of a module that `annorex rex install` wrote. A new install replaces only a
/// file with this line.
pub const MODULE_MARKER: &str = "-- Installed by `annorex rex install`";

/// The file name of the module. `require("annorex")` loads it.
pub const MODULE_FILE: &str = "annorex.lua";

/// The line in the module source that `module_contents` replaces with the path of the binary.
pub const INSTALLED_PLACEHOLDER: &str = "local INSTALLED_BIN = nil -- set by annorex rex install\n";

/// The module as installed: the marker line with the version, then the source. `bin` is the
/// binary that Rex starts when `setup{}` gives no `bin`.
pub fn module_contents(bin: Option<&Path>) -> String {
    // A path that is not UTF-8 cannot be written exactly. The module then uses the known places.
    let installed = match bin.and_then(Path::to_str) {
        Some(bin) => format!("local INSTALLED_BIN = {}\n", lua_string(bin)),
        None => "local INSTALLED_BIN = nil\n".to_owned(),
    };
    format!(
        "{MODULE_MARKER} (annorex {}); run it again after you upgrade Annorex.\n{}",
        env!("CARGO_PKG_VERSION"),
        MODULE_SOURCE.replacen(INSTALLED_PLACEHOLDER, &installed, 1)
    )
}

/// `text` as a Lua string literal in double quotes.
fn lua_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// The directory of the `init.lua` that Rex reads: the directory of `REX_CONFIG`, else
/// `XDG_CONFIG_HOME/rex`, else `HOME/.config/rex`. Empty values count as not set.
pub fn config_dir(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let get = |key: &str| env(key).filter(|value| !value.is_empty()).map(PathBuf::from);
    if let Some(config) = get("REX_CONFIG") {
        return config.parent().map(Path::to_path_buf);
    }
    if let Some(xdg) = get("XDG_CONFIG_HOME") {
        return Some(xdg.join("rex"));
    }
    get("HOME").map(|home| home.join(".config").join("rex"))
}

/// The `init.lua` that Rex reads: `REX_CONFIG` itself, else `init.lua` in `config_dir`. Empty
/// values count as not set.
pub fn config_file(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(config) = env("REX_CONFIG").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(config));
    }
    config_dir(env).map(|dir| dir.join("init.lua"))
}

/// The result of an install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub path: PathBuf,
    /// True when an earlier install of Annorex was replaced.
    pub replaced: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("{} exists and Annorex did not write it; remove or rename it first", path.display())]
    Foreign { path: PathBuf },
    #[error("cannot write the module: {0}")]
    Io(#[from] std::io::Error),
}

/// Write the module to `dir`, with `bin` as the installed binary. Create `dir` if necessary.
/// Replace only a file that an earlier install wrote. Never open `init.lua`: `crate::setup` does that.
pub fn install(dir: &Path, bin: Option<&Path>) -> Result<Installed, InstallError> {
    let path = dir.join(MODULE_FILE);
    let replaced = match std::fs::read_to_string(&path) {
        Ok(existing) => {
            if !existing.lines().next().is_some_and(|line| line.starts_with(MODULE_MARKER)) {
                return Err(InstallError::Foreign { path });
            }
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    std::fs::create_dir_all(dir)?;
    write_atomic(&path, &module_contents(bin))?;
    Ok(Installed { path, replaced })
}

/// The path of the binary to write into the module. A Homebrew binary runs from
/// `<prefix>/Cellar/annorex/<version>/bin/annorex`, and an upgrade removes that folder. So when
/// `<prefix>/bin/annorex` exists, use it: it stays the same across upgrades. Else use `exe`.
pub fn stable_bin(exe: &Path) -> PathBuf {
    let Ok(canonical) = exe.canonicalize() else {
        return exe.to_path_buf();
    };
    let name = |path: Option<&Path>, expected: &str| {
        path.and_then(Path::file_name).is_some_and(|name| name == expected)
    };
    let bin_dir = canonical.parent();
    let formula = bin_dir.and_then(Path::parent).and_then(Path::parent);
    let cellar = formula.and_then(Path::parent);
    if name(Some(&canonical), "annorex")
        && name(bin_dir, "bin")
        && name(formula, "annorex")
        && name(cellar, "Cellar")
        && let Some(prefix) = cellar.and_then(Path::parent)
    {
        let stable = prefix.join("bin").join("annorex");
        if stable.exists() {
            return stable;
        }
    }
    exe.to_path_buf()
}

/// The version in the marker line of an installed module, for example `0.1.0`.
pub fn installed_version(contents: &str) -> Option<&str> {
    let rest = contents.lines().next()?.strip_prefix(MODULE_MARKER)?.strip_prefix(" (annorex ")?;
    rest.split_once(')').map(|(version, _)| version)
}

/// The result of a module refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    /// The module has the version of this binary.
    Current,
    /// The module had another version. The refresh wrote the module of this binary.
    Updated,
    /// There is no module.
    Absent,
    /// The module does not have the marker line. The refresh did not change it.
    Foreign,
}

/// Write the module of this binary to `dir` when an earlier install wrote a module of another
/// version. Never create a missing module, never change a module without the marker, and never
/// open `init.lua`.
pub fn refresh(dir: &Path, bin: Option<&Path>) -> Result<Refresh, InstallError> {
    let path = dir.join(MODULE_FILE);
    let existing = match std::fs::read_to_string(&path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Refresh::Absent),
        Err(error) => return Err(error.into()),
    };
    if !existing.lines().next().is_some_and(|line| line.starts_with(MODULE_MARKER)) {
        return Ok(Refresh::Foreign);
    }
    if installed_version(&existing) == Some(env!("CARGO_PKG_VERSION")) {
        return Ok(Refresh::Current);
    }
    write_atomic(&path, &module_contents(bin))?;
    Ok(Refresh::Updated)
}

/// Write `contents` to a temporary file beside `path`, then rename it to `path`. So Rex never
/// reads a module that is only partly written.
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    write_atomic_with_mode(path, contents, None)
}

/// `write_atomic` that gives the new file the permission bits `mode`, before the rename. So a
/// private file never shows its new text with other permissions.
pub(crate) fn write_atomic_with_mode(path: &Path, contents: &str, mode: Option<u32>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let written = std::fs::write(&temp, contents)
        .and_then(|()| match mode {
            Some(mode) => std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode & 0o7777)),
            None => Ok(()),
        })
        .and_then(|()| std::fs::rename(&temp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

/// The outcome of one `rex config check` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub success: bool,
    /// What Rex printed, on failure.
    pub message: String,
}

/// The result of the module check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    Passed,
    Failed(String),
    /// Rex could not be run, so nothing was checked.
    Skipped,
    /// The private scratch folder could not be created, so nothing was checked.
    NoScratch,
}

impl Check {
    /// The line that `rex install` prints for this result.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Passed => "Checked the module with rex config check: no errors.".to_owned(),
            Self::Skipped => "Skipped the module check: rex is not available.".to_owned(),
            Self::NoScratch => "Skipped the module check: cannot create a private scratch folder.".to_owned(),
            Self::Failed(message) => format!("rex config check found errors in the module:\n{message}"),
        }
    }
}

/// Check the module with Rex before the user reloads: write the module and an `init.lua` that
/// loads it into `scratch`, then call `run` with the path of that `init.lua`. The real config
/// is not touched.
pub fn check_module(
    scratch: &Path,
    bin: Option<&Path>,
    run: impl FnOnce(&Path) -> std::io::Result<Probe>,
) -> Check {
    let init = scratch.join("init.lua");
    let written = std::fs::write(scratch.join(MODULE_FILE), module_contents(bin))
        .and_then(|()| std::fs::write(&init, "require(\"annorex\").setup{}\n"));
    if written.is_err() {
        return Check::Skipped;
    }
    match run(&init) {
        Ok(probe) if probe.success => Check::Passed,
        Ok(probe) => Check::Failed(probe.message),
        Err(_) => Check::Skipped,
    }
}

/// Create the scratch folder for the module check. The folder is private (mode 0700). The call
/// fails if `path` exists, so a folder that Annorex did not create is never used or removed.
pub fn make_scratch(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(false).mode(0o700).create(path)
}

/// Run `rex config check` on `init` with `REX_CONFIG` set to it, for this call only. `rex` is the binary
/// that the other commands use.
pub fn run_rex_config_check(rex: &Path, init: &Path) -> std::io::Result<Probe> {
    let output = std::process::Command::new(rex)
        .args(["config", "check"])
        .arg(init)
        .env("REX_CONFIG", init)
        .output()?;
    let mut message = String::from_utf8_lossy(&output.stdout).into_owned();
    message.push_str(&String::from_utf8_lossy(&output.stderr));
    let message = message
        .lines()
        .filter(|line| line.starts_with("error") || line.starts_with("Error"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Probe { success: output.status.success(), message })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| OsString::from(v))
    }

    #[test]
    fn make_scratch_creates_a_private_folder() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("check");
        make_scratch(&path).expect("make scratch");
        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn make_scratch_fails_if_the_path_exists() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("check");
        std::fs::create_dir(&path).expect("folder");
        std::fs::write(path.join("keep.txt"), "x").expect("file");
        assert!(make_scratch(&path).is_err());
        assert!(path.join("keep.txt").exists());
    }

    #[test]
    fn make_scratch_fails_on_a_symbolic_link() {
        let dir = tempfile::tempdir().expect("temp dir");
        let target = dir.path().join("target");
        std::fs::create_dir(&target).expect("target");
        std::fs::write(target.join("keep.txt"), "x").expect("file");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("link");
        assert!(make_scratch(&link).is_err());
        assert!(target.join("keep.txt").exists());
        let dangling = dir.path().join("dangling");
        std::os::unix::fs::symlink(dir.path().join("missing"), &dangling).expect("link");
        assert!(make_scratch(&dangling).is_err());
        assert!(dangling.symlink_metadata().is_ok());
        assert!(!dir.path().join("missing").exists());
    }

    #[test]
    fn check_summary_tells_a_scratch_failure_from_a_missing_rex() {
        assert!(Check::NoScratch.summary().contains("private scratch folder"));
        assert!(Check::Skipped.summary().contains("rex is not available"));
    }

    #[test]
    fn config_dir_prefers_rex_config_parent() {
        let pairs = [("REX_CONFIG", "/etc/rex/main.lua"), ("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")];
        assert_eq!(config_dir(env(&pairs)), Some(PathBuf::from("/etc/rex")));
    }

    #[test]
    fn config_dir_uses_xdg_then_home() {
        assert_eq!(
            config_dir(env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")])),
            Some(PathBuf::from("/x/rex"))
        );
        assert_eq!(config_dir(env(&[("HOME", "/h")])), Some(PathBuf::from("/h/.config/rex")));
        assert_eq!(config_dir(env(&[])), None);
    }

    #[test]
    fn config_dir_ignores_empty_values() {
        let pairs = [("REX_CONFIG", ""), ("XDG_CONFIG_HOME", ""), ("HOME", "/h")];
        assert_eq!(config_dir(env(&pairs)), Some(PathBuf::from("/h/.config/rex")));
    }

    #[test]
    fn install_writes_marker_first_line() {
        let dir = tempfile::tempdir().expect("temp dir");
        let target = dir.path().join("new");
        let installed = install(&target, None).expect("install");
        assert_eq!(installed.path, target.join("annorex.lua"));
        assert!(!installed.replaced);
        let text = std::fs::read_to_string(&installed.path).expect("module");
        let first = text.lines().next().expect("first line");
        assert!(first.starts_with(MODULE_MARKER), "{first}");
        assert!(first.contains(env!("CARGO_PKG_VERSION")), "{first}");
        assert_eq!(text, module_contents(None));
    }

    #[test]
    fn install_replaces_own_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("annorex.lua");
        std::fs::write(&path, format!("{MODULE_MARKER} (annorex 0.0.1)\nold\n")).expect("old module");
        let installed = install(dir.path(), None).expect("install");
        assert!(installed.replaced);
        assert_eq!(std::fs::read_to_string(&path).expect("module"), module_contents(None));
    }

    #[test]
    fn install_refuses_foreign_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("annorex.lua");
        std::fs::write(&path, "-- my own annorex.lua\n").expect("foreign");
        match install(dir.path(), None) {
            Err(InstallError::Foreign { path: found }) => assert_eq!(found, path),
            other => panic!("expected Foreign, got {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(&path).expect("kept"), "-- my own annorex.lua\n");
    }

    #[test]
    fn config_file_is_rex_config_else_init_lua_in_the_config_dir() {
        let pairs = [("REX_CONFIG", "/etc/rex/main.lua"), ("HOME", "/h")];
        assert_eq!(config_file(env(&pairs)), Some(PathBuf::from("/etc/rex/main.lua")));
        let pairs = [("REX_CONFIG", ""), ("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")];
        assert_eq!(config_file(env(&pairs)), Some(PathBuf::from("/x/rex/init.lua")));
        assert_eq!(config_file(env(&[("HOME", "/h")])), Some(PathBuf::from("/h/.config/rex/init.lua")));
        assert_eq!(config_file(env(&[])), None);
    }

    /// `rex install` changes `init.lua` through `crate::setup`. The module install alone never does.
    #[test]
    fn install_alone_never_touches_init_lua() {
        let dir = tempfile::tempdir().expect("temp dir");
        let init = dir.path().join("init.lua");
        std::fs::write(&init, "rex.bind(\"cmd+d\", \"pane.split\")\n").expect("init");
        let before = std::fs::metadata(&init).and_then(|m| m.modified()).expect("mtime");
        std::thread::sleep(std::time::Duration::from_millis(20));
        install(dir.path(), None).expect("install");
        assert_eq!(std::fs::read_to_string(&init).expect("init"), "rex.bind(\"cmd+d\", \"pane.split\")\n");
        assert_eq!(std::fs::metadata(&init).and_then(|m| m.modified()).expect("mtime"), before);
    }

    #[test]
    fn module_defines_annorex_last_and_no_bind() {
        assert!(MODULE_SOURCE.contains("name = \"annorex_last\""));
        let code = MODULE_SOURCE.lines().filter(|line| !line.trim_start().starts_with("--"));
        assert!(code.clone().all(|line| !line.contains("rex.bind(")), "the module must not bind keys");
        assert!(code.clone().any(|line| line.contains("rex.action{")));
        assert!(MODULE_SOURCE.contains("\"rex\", \"open\", view"));
        assert!(MODULE_SOURCE.contains("open_view(ctx, \"last\")"));
    }

    #[test]
    fn module_has_no_placement_option() {
        assert!(!MODULE_SOURCE.contains("placement"), "Annorex opens over the full window only");
        assert!(!MODULE_SOURCE.contains("--placement"));
    }

    #[test]
    fn lua_module_registers_both_actions() {
        assert!(MODULE_SOURCE.contains("name = \"annorex_last\""));
        assert!(MODULE_SOURCE.contains("name = \"annorex_files\""));
        assert!(MODULE_SOURCE.contains("title = \"Annorex: review files\""));
        assert!(MODULE_SOURCE.contains("open_view(ctx, \"files\")"));
        assert!(MODULE_SOURCE.contains("rex.bind(\"cmd+shift+f\", \"annorex_files\")"));
        let code = MODULE_SOURCE.lines().filter(|line| !line.trim_start().starts_with("--"));
        assert!(code.clone().all(|line| !line.contains("rex.bind(")), "the module must not bind keys");
    }

    #[test]
    fn install_replaces_the_old_module_version() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("annorex.lua");
        let old = format!("{MODULE_MARKER} (annorex 0.1.0); run it again.\nname = \"annorex_last\"\n");
        std::fs::write(&path, old).expect("old module");
        let installed = install(dir.path(), None).expect("install");
        assert!(installed.replaced);
        let text = std::fs::read_to_string(&path).expect("module");
        assert!(text.contains("annorex_files"), "the new module has the files action");
    }

    #[test]
    fn module_contents_puts_the_installed_path_in_the_placeholder() {
        assert_eq!(MODULE_SOURCE.matches(INSTALLED_PLACEHOLDER).count(), 1, "one placeholder line");
        let text = module_contents(Some(Path::new("/Users/me/.cargo/bin/annorex")));
        assert!(text.contains("local INSTALLED_BIN = \"/Users/me/.cargo/bin/annorex\"\n"), "{text}");
        assert!(!text.contains(INSTALLED_PLACEHOLDER));
        let none = module_contents(None);
        assert!(none.contains("local INSTALLED_BIN = nil\n"), "{none}");
    }

    #[test]
    fn a_path_that_is_not_utf8_is_not_written() {
        use std::os::unix::ffi::OsStrExt as _;
        let bad = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff/annorex"));
        assert!(module_contents(Some(bad)).contains("local INSTALLED_BIN = nil\n"));
    }

    #[test]
    fn check_module_uses_the_installed_path() {
        let scratch = tempfile::tempdir().expect("temp dir");
        let bin = Path::new("/opt/x/annorex");
        let mut module = String::new();
        check_module(scratch.path(), Some(bin), |init| {
            module = std::fs::read_to_string(init.with_file_name("annorex.lua")).expect("module");
            Ok(Probe { success: true, message: String::new() })
        });
        assert_eq!(module, module_contents(Some(bin)));
    }

    #[test]
    fn module_checks_that_a_found_path_is_a_file() {
        assert!(
            MODULE_SOURCE.contains("io.open(path .. \"/.\", \"r\")"),
            "a directory must not count as found"
        );
    }

    #[test]
    fn lua_string_escapes_backslash_quote_and_line_breaks() {
        assert_eq!(lua_string("/a b/c"), "\"/a b/c\"");
        assert_eq!(lua_string("a\"b\\c\nd\re"), "\"a\\\"b\\\\c\\nd\\re\"");
    }

    #[test]
    fn module_looks_in_the_known_places_in_order() {
        let order = [
            "\"/.cargo/bin/annorex\"",
            "\"/opt/homebrew/bin/annorex\"",
            "\"/usr/local/bin/annorex\"",
            "\"/.local/bin/annorex\"",
        ];
        let positions: Vec<usize> = order
            .iter()
            .map(|needle| MODULE_SOURCE.find(needle).unwrap_or_else(|| panic!("{needle}")))
            .collect();
        assert!(positions.is_sorted(), "{positions:?}");
        assert!(MODULE_SOURCE.contains("os.getenv(\"CARGO_HOME\")"));
        assert!(MODULE_SOURCE.contains("annorex was not found; tried: "));
    }

    #[test]
    fn check_passes_when_rex_accepts_the_scratch_config() {
        let scratch = tempfile::tempdir().expect("temp dir");
        let seen = std::cell::RefCell::new(None);
        let result = check_module(scratch.path(), None, |init| {
            *seen.borrow_mut() = Some((
                std::fs::read_to_string(init).expect("init"),
                std::fs::read_to_string(init.with_file_name("annorex.lua")).expect("module"),
            ));
            Ok(Probe { success: true, message: String::new() })
        });
        assert_eq!(result, Check::Passed);
        let (init, module) = seen.into_inner().expect("runner called");
        assert!(init.contains("require(\"annorex\").setup{}"), "{init}");
        assert_eq!(module, module_contents(None));
    }

    #[test]
    fn check_fails_with_the_rex_message() {
        let scratch = tempfile::tempdir().expect("temp dir");
        let result = check_module(scratch.path(), None, |_| {
            Ok(Probe { success: false, message: "error annorex.lua:3: boom".to_owned() })
        });
        assert_eq!(result, Check::Failed("error annorex.lua:3: boom".to_owned()));
    }

    /// A Homebrew layout in a temporary folder: `T/Cellar/annorex/0.2.0/bin/annorex`. The folder
    /// path is canonical, so it compares with the canonical path of the binary.
    fn cellar() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().canonicalize().expect("canonical");
        let exe = root.join("Cellar/annorex/0.2.0/bin/annorex");
        std::fs::create_dir_all(exe.parent().expect("parent")).expect("cellar");
        std::fs::write(&exe, "").expect("binary");
        (dir, root, exe)
    }

    #[test]
    fn stable_bin_maps_cellar_to_prefix_bin() {
        let (_dir, root, exe) = cellar();
        assert_eq!(stable_bin(&exe), exe, "without <prefix>/bin/annorex, keep the input");
        std::fs::create_dir_all(root.join("bin")).expect("bin");
        std::os::unix::fs::symlink(&exe, root.join("bin/annorex")).expect("link");
        assert_eq!(stable_bin(&exe), root.join("bin/annorex"));
    }

    #[test]
    fn stable_bin_keeps_a_path_outside_the_cellar() {
        let dir = tempfile::tempdir().expect("temp dir");
        let exe = dir.path().join("bin/annorex");
        std::fs::create_dir_all(exe.parent().expect("parent")).expect("bin");
        std::fs::write(&exe, "").expect("binary");
        assert_eq!(stable_bin(&exe), exe);
        let missing = Path::new("/nonexistent/Cellar/annorex/0.2.0/bin/annorex");
        assert_eq!(stable_bin(missing), missing);
    }

    #[test]
    fn rex_install_writes_the_stable_bin() {
        let (_dir, root, exe) = cellar();
        std::fs::create_dir_all(root.join("bin")).expect("bin");
        std::os::unix::fs::symlink(&exe, root.join("bin/annorex")).expect("link");
        let text = module_contents(Some(&stable_bin(&exe)));
        let line = format!("local INSTALLED_BIN = \"{}\"\n", root.join("bin/annorex").display());
        assert!(text.contains(&line), "{text}");
        assert!(!text.contains("Cellar"), "{text}");
    }

    #[test]
    fn installed_version_reads_the_marker_line() {
        assert_eq!(installed_version(&module_contents(None)), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(installed_version(&format!("{MODULE_MARKER} (annorex 0.0.1); x\n")), Some("0.0.1"));
        assert_eq!(installed_version("-- my own annorex.lua (annorex 0.0.1)\n"), None);
        assert_eq!(installed_version(""), None);
    }

    #[test]
    fn refresh_of_a_missing_module_is_absent() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(matches!(refresh(dir.path(), None), Ok(Refresh::Absent)));
        assert!(!dir.path().join(MODULE_FILE).exists(), "the refresh never creates the module");
        let missing = dir.path().join("missing");
        assert!(matches!(refresh(&missing, None), Ok(Refresh::Absent)));
        assert!(!missing.exists());
    }

    #[test]
    fn refresh_of_the_same_version_is_current() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(MODULE_FILE);
        let text = module_contents(Some(Path::new("/opt/x/annorex")));
        std::fs::write(&path, &text).expect("module");
        assert!(matches!(refresh(dir.path(), None), Ok(Refresh::Current)));
        assert_eq!(std::fs::read_to_string(&path).expect("module"), text);
    }

    #[test]
    fn refresh_updates_another_version() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(MODULE_FILE);
        let current = format!("(annorex {})", env!("CARGO_PKG_VERSION"));
        let old = module_contents(None).replacen(&current, "(annorex 0.0.1)", 1);
        std::fs::write(&path, old).expect("old module");
        let bin = Path::new("/opt/homebrew/bin/annorex");
        assert!(matches!(refresh(dir.path(), Some(bin)), Ok(Refresh::Updated)));
        assert_eq!(std::fs::read_to_string(&path).expect("module"), module_contents(Some(bin)));
    }

    #[test]
    fn refresh_leaves_a_foreign_module() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(MODULE_FILE);
        std::fs::write(&path, "-- my own annorex.lua\n").expect("foreign");
        assert!(matches!(refresh(dir.path(), None), Ok(Refresh::Foreign)));
        assert_eq!(std::fs::read_to_string(&path).expect("kept"), "-- my own annorex.lua\n");
    }

    #[test]
    fn install_and_refresh_leave_no_temporary_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        install(dir.path(), None).expect("install");
        let current = format!("(annorex {})", env!("CARGO_PKG_VERSION"));
        let path = dir.path().join(MODULE_FILE);
        let old = module_contents(None).replacen(&current, "(annorex 0.0.1)", 1);
        std::fs::write(&path, old).expect("old module");
        assert!(matches!(refresh(dir.path(), None), Ok(Refresh::Updated)));
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .expect("dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, [MODULE_FILE], "only the module stays");
        assert_eq!(
            write_atomic(&dir.path().join("missing/x.lua"), "x").map_err(|e| e.kind()),
            Err(std::io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn check_is_skipped_when_rex_is_missing() {
        let scratch = tempfile::tempdir().expect("temp dir");
        let result = check_module(scratch.path(), None, |_| Err(std::io::ErrorKind::NotFound.into()));
        assert_eq!(result, Check::Skipped);
    }
}
