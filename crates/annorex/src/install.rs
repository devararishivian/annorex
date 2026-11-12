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

/// The module as installed: the marker line with the version, then the source.
pub fn module_contents() -> String {
    format!(
        "{MODULE_MARKER} (annorex {}); run it again after you upgrade Annorex.\n{MODULE_SOURCE}",
        env!("CARGO_PKG_VERSION")
    )
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

/// Write the module to `dir`. Create `dir` if necessary. Replace only a file that an earlier
/// install wrote. Never open `init.lua`.
pub fn install(dir: &Path) -> Result<Installed, InstallError> {
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
    std::fs::write(&path, module_contents())?;
    Ok(Installed { path, replaced })
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
}

/// Check the module with Rex before the user reloads: write the module and an `init.lua` that
/// loads it into `scratch`, then call `run` with the path of that `init.lua`. The real config
/// is not touched.
pub fn check_module(scratch: &Path, run: impl FnOnce(&Path) -> std::io::Result<Probe>) -> Check {
    let init = scratch.join("init.lua");
    let written = std::fs::write(scratch.join(MODULE_FILE), module_contents())
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
        let installed = install(&target).expect("install");
        assert_eq!(installed.path, target.join("annorex.lua"));
        assert!(!installed.replaced);
        let text = std::fs::read_to_string(&installed.path).expect("module");
        let first = text.lines().next().expect("first line");
        assert!(first.starts_with(MODULE_MARKER), "{first}");
        assert!(first.contains(env!("CARGO_PKG_VERSION")), "{first}");
        assert!(text.ends_with(MODULE_SOURCE));
    }

    #[test]
    fn install_replaces_own_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("annorex.lua");
        std::fs::write(&path, format!("{MODULE_MARKER} (annorex 0.0.1)\nold\n")).expect("old module");
        let installed = install(dir.path()).expect("install");
        assert!(installed.replaced);
        assert_eq!(std::fs::read_to_string(&path).expect("module"), module_contents());
    }

    #[test]
    fn install_refuses_foreign_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("annorex.lua");
        std::fs::write(&path, "-- my own annorex.lua\n").expect("foreign");
        match install(dir.path()) {
            Err(InstallError::Foreign { path: found }) => assert_eq!(found, path),
            other => panic!("expected Foreign, got {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(&path).expect("kept"), "-- my own annorex.lua\n");
    }

    #[test]
    fn install_never_touches_init_lua() {
        let dir = tempfile::tempdir().expect("temp dir");
        let init = dir.path().join("init.lua");
        std::fs::write(&init, "rex.bind(\"cmd+d\", \"pane.split\")\n").expect("init");
        let before = std::fs::metadata(&init).and_then(|m| m.modified()).expect("mtime");
        std::thread::sleep(std::time::Duration::from_millis(20));
        install(dir.path()).expect("install");
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
        let installed = install(dir.path()).expect("install");
        assert!(installed.replaced);
        let text = std::fs::read_to_string(&path).expect("module");
        assert!(text.contains("annorex_files"), "the new module has the files action");
    }

    #[test]
    fn check_passes_when_rex_accepts_the_scratch_config() {
        let scratch = tempfile::tempdir().expect("temp dir");
        let seen = std::cell::RefCell::new(None);
        let result = check_module(scratch.path(), |init| {
            *seen.borrow_mut() = Some((
                std::fs::read_to_string(init).expect("init"),
                std::fs::read_to_string(init.with_file_name("annorex.lua")).expect("module"),
            ));
            Ok(Probe { success: true, message: String::new() })
        });
        assert_eq!(result, Check::Passed);
        let (init, module) = seen.into_inner().expect("runner called");
        assert!(init.contains("require(\"annorex\").setup{}"), "{init}");
        assert_eq!(module, module_contents());
    }

    #[test]
    fn check_fails_with_the_rex_message() {
        let scratch = tempfile::tempdir().expect("temp dir");
        let result = check_module(scratch.path(), |_| {
            Ok(Probe { success: false, message: "error annorex.lua:3: boom".to_owned() })
        });
        assert_eq!(result, Check::Failed("error annorex.lua:3: boom".to_owned()));
    }

    #[test]
    fn check_is_skipped_when_rex_is_missing() {
        let scratch = tempfile::tempdir().expect("temp dir");
        let result = check_module(scratch.path(), |_| Err(std::io::ErrorKind::NotFound.into()));
        assert_eq!(result, Check::Skipped);
    }
}
