//! The config file `config.toml`.
//!
//! The file is optional. Every key in it is optional too. A wrong key stops the command with a
//! message that names the file and the key, so a typing mistake never passes without notice.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::deliver::ENTER_DELAY;
use crate::keys::KeyMap;
use crate::open::Placement;

/// The labels that exist before the config file adds more.
const DEFAULT_LABELS: [&str; 3] = ["question", "looks good", "must fix"];
/// The range of `enter_delay_ms`.
const DELAY_RANGE_MS: std::ops::RangeInclusive<u64> = 50..=5000;

/// A problem with the config file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read the config file {}: {source}", path.display())]
    Read { path: PathBuf, source: std::io::Error },
    #[error("config file {}: {message}", path.display())]
    Invalid { path: PathBuf, message: String },
}

/// The settings of Annorex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Where the viewer opens. `None` means that the command line or the default decides.
    pub placement: Option<Placement>,
    /// The wait between the paste and the Enter key.
    pub enter_delay: Duration,
    /// The labels of an annotation, the default labels first.
    pub labels: Vec<String>,
    /// The keys of the viewer.
    pub keys: KeyMap,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            placement: None,
            enter_delay: ENTER_DELAY,
            labels: DEFAULT_LABELS.iter().map(|label| (*label).to_owned()).collect(),
            keys: KeyMap::default(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    placement: Option<Placement>,
    enter_delay_ms: Option<u64>,
    labels: Option<Vec<String>>,
    keys: Option<BTreeMap<String, String>>,
}

/// The path of the config file: `$XDG_CONFIG_HOME/annorex/config.toml` when that variable is an
/// absolute path, else `$HOME/.config/annorex/config.toml`.
pub fn config_path(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let base = env("XDG_CONFIG_HOME").map(PathBuf::from).filter(|path| path.is_absolute()).or_else(|| {
        env("HOME").map(PathBuf::from).filter(|path| path.is_absolute()).map(|home| home.join(".config"))
    })?;
    Some(base.join("annorex").join("config.toml"))
}

/// The placement: the command line flag, else the config file, else `auto`.
pub fn resolve_placement(flag: Option<Placement>, config: Option<Placement>) -> Placement {
    flag.or(config).unwrap_or_default()
}

impl Config {
    /// Load the config file that the environment names. No path or no file gives the defaults.
    pub fn from_env(env: impl Fn(&str) -> Option<OsString>) -> Result<Self, ConfigError> {
        config_path(env).map_or_else(|| Ok(Self::default()), |path| Self::load(&path))
    }

    /// Load one config file. A missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => return Err(ConfigError::Read { path: path.to_path_buf(), source }),
        };
        let invalid = |message: String| ConfigError::Invalid { path: path.to_path_buf(), message };
        let raw: Raw = toml::from_str(&text).map_err(|error| invalid(error.message().to_owned()))?;
        let mut config = Self { placement: raw.placement, ..Self::default() };
        if let Some(ms) = raw.enter_delay_ms {
            if !DELAY_RANGE_MS.contains(&ms) {
                return Err(invalid(format!(
                    "enter_delay_ms is {ms}, but it must be from {} to {}",
                    DELAY_RANGE_MS.start(),
                    DELAY_RANGE_MS.end()
                )));
            }
            config.enter_delay = Duration::from_millis(ms);
        }
        for label in raw.labels.unwrap_or_default() {
            if label.trim().is_empty() {
                return Err(invalid("labels must not hold an empty label".to_owned()));
            }
            if !config.labels.contains(&label) {
                config.labels.push(label);
            }
        }
        if let Some(keys) = raw.keys {
            config.keys = KeyMap::with_overrides(&keys).map_err(invalid)?;
        }
        Ok(config)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::{Config, config_path, resolve_placement};
    use crate::open::Placement;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| pairs.iter().find(|(k, _)| *k == key).map(|(_, value)| OsString::from(value))
    }

    fn load(text: &str) -> Result<Config, String> {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).expect("config");
        Config::load(&path).map_err(|error| error.to_string())
    }

    #[test]
    fn config_path_uses_xdg_then_home() {
        let xdg = env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")]);
        assert_eq!(config_path(xdg), Some(PathBuf::from("/x/annorex/config.toml")));
        assert_eq!(
            config_path(env(&[("HOME", "/h")])),
            Some(PathBuf::from("/h/.config/annorex/config.toml"))
        );
        assert_eq!(config_path(env(&[])), None);
    }

    #[test]
    fn config_path_ignores_a_relative_or_empty_xdg_value() {
        let relative = env(&[("XDG_CONFIG_HOME", "rel"), ("HOME", "/h")]);
        assert_eq!(config_path(relative), Some(PathBuf::from("/h/.config/annorex/config.toml")));
        let empty = env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/h")]);
        assert_eq!(config_path(empty), Some(PathBuf::from("/h/.config/annorex/config.toml")));
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().expect("dir");
        let config = Config::load(&dir.path().join("nope.toml")).expect("defaults");
        assert_eq!(config, Config::default());
        assert_eq!(config.placement, None);
        assert_eq!(config.enter_delay, Duration::from_millis(300));
        assert_eq!(config.labels, ["question", "looks good", "must fix"]);
    }

    #[test]
    fn an_empty_file_gives_defaults() {
        assert_eq!(load("").expect("empty"), Config::default());
    }

    #[test]
    fn labels_are_added_after_the_defaults() {
        let config = load("labels = [\"needs test\"]\n").expect("config");
        assert_eq!(config.labels, ["question", "looks good", "must fix", "needs test"]);
    }

    #[test]
    fn placement_and_delay_are_read() {
        let config = load("placement = \"below\"\nenter_delay_ms = 120\n").expect("config");
        assert_eq!(config.placement, Some(Placement::Below));
        assert_eq!(config.enter_delay, Duration::from_millis(120));
    }

    #[test]
    fn unknown_key_names_the_key_and_the_path() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "colour = \"red\"\n").expect("config");
        let error = Config::load(&path).expect_err("unknown key").to_string();
        assert!(error.contains("colour"), "{error}");
        assert!(error.contains(&path.display().to_string()), "{error}");
    }

    #[test]
    fn unknown_action_names_the_key() {
        let error = load("[keys]\nfly = \"f\"\n").expect_err("unknown action");
        assert!(error.contains("fly"), "{error}");
    }

    #[test]
    fn same_key_twice_names_both_actions() {
        let error = load("[keys]\ncomment = \"d\"\n").expect_err("same key");
        assert!(error.contains("comment") && error.contains("delete"), "{error}");
    }

    #[test]
    fn a_key_override_is_read() {
        let config = load("[keys]\ncomment = \"y\"\n").expect("config");
        assert_ne!(config.keys, Config::default().keys);
    }

    #[test]
    fn delay_out_of_range_is_an_error() {
        for value in ["10", "6000"] {
            let error = load(&format!("enter_delay_ms = {value}\n")).expect_err(value);
            assert!(error.contains("enter_delay_ms"), "{error}");
        }
    }

    #[test]
    fn a_bad_placement_names_the_value() {
        let error = load("placement = \"left\"\n").expect_err("bad placement");
        assert!(error.contains("left"), "{error}");
    }

    #[test]
    fn a_file_that_is_not_toml_names_the_path() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "= = =\n").expect("config");
        let error = Config::load(&path).expect_err("bad toml").to_string();
        assert!(error.contains(&path.display().to_string()), "{error}");
    }

    #[test]
    fn flag_wins_over_config_placement() {
        assert_eq!(resolve_placement(Some(Placement::Below), Some(Placement::Right)), Placement::Below);
        assert_eq!(resolve_placement(None, Some(Placement::Right)), Placement::Right);
        assert_eq!(resolve_placement(None, None), Placement::Auto);
    }

    #[test]
    fn from_env_without_a_path_gives_defaults() {
        assert_eq!(Config::from_env(env(&[])).expect("defaults"), Config::default());
        let _ = Path::new("");
    }
}
