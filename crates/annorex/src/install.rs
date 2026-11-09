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
        assert!(MODULE_SOURCE.contains("\"rex\", \"open\", \"last\""));
    }

    #[test]
    fn module_passes_placement_only_when_set() {
        let code: String = MODULE_SOURCE
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        let guard = code.find("if M.options.placement").expect("a guard for placement");
        let flag = code.find("\"--placement\"").expect("the flag");
        assert!(guard < flag, "the flag must come after the guard");
        assert!(!code.contains("placement = \"auto\""), "the module must not set a default placement");
    }
}
