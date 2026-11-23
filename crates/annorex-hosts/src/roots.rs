// Derived from plannotator-tui (https://github.com/plannotator/plannotator-tui) @ 83b0cd9, Copyright (c) 2026 Michael Ramos, MIT License. Changes for Annorex: moved from crates/plannotator-tui/src/last/roots.rs into this crate and made public; kept only Claude Code, Pi, OpenCode, and Codex; the Pi agent directory default is fixed to `.pi/agent`; tests moved to tests/roots.rs; added `codex_sessions`.
//! Host storage roots derived from the process environment.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::opencode;

/// The directory where Pi keeps its agent state, relative to the home directory.
pub const PI_DEFAULT_AGENT_DIR: &str = ".pi/agent";

/// The folder of `OhMyPi` in the home directory when `PI_CONFIG_DIR` is not set.
pub const OMP_DEFAULT_CONFIG_DIR: &str = ".omp";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    home: PathBuf,
    claude_config: PathBuf,
    pi_session_dir: Option<PathBuf>,
    pi_agent_dir: Option<PathBuf>,
    omp_config_dir: Option<PathBuf>,
    opencode_db: Option<PathBuf>,
    codex_home: PathBuf,
    xdg_data_home: PathBuf,
}

impl Roots {
    pub fn from_env() -> Self {
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        Self::from_lookup(home, |key| std::env::var_os(key))
    }

    pub fn from_lookup(home: PathBuf, env: impl Fn(&str) -> Option<OsString>) -> Self {
        let path = |key: &str| env(key).filter(|value| !value.is_empty()).map(PathBuf::from);
        let xdg_data_home = path("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share"));
        Self {
            claude_config: path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
            pi_session_dir: path("PI_CODING_AGENT_SESSION_DIR"),
            pi_agent_dir: path("PI_CODING_AGENT_DIR"),
            omp_config_dir: path("PI_CONFIG_DIR"),
            opencode_db: path("OPENCODE_DB"),
            codex_home: path("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
            xdg_data_home,
            home,
        }
    }

    /// The home directory that the roots use.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Claude Code's configuration directory: `sessions/` and `projects/` live here.
    pub fn claude_config(&self) -> &Path {
        &self.claude_config
    }

    pub fn pi_sessions(&self) -> PathBuf {
        self.pi_session_dir.clone().unwrap_or_else(|| {
            self.pi_agent_dir.clone().unwrap_or_else(|| self.home.join(PI_DEFAULT_AGENT_DIR)).join("sessions")
        })
    }

    /// `OhMyPi` sessions of the default profile: `$PI_CODING_AGENT_DIR/sessions`, else
    /// `~/$PI_CONFIG_DIR/agent/sessions`, else `~/.omp/agent/sessions`. `PI_CONFIG_DIR` is a name
    /// under home, so an absolute value also goes under home. The profile and XDG rules are not read.
    pub fn omp_sessions(&self) -> PathBuf {
        let agent = self.pi_agent_dir.clone().unwrap_or_else(|| {
            let config = self
                .omp_config_dir
                .as_deref()
                .map_or(Path::new(OMP_DEFAULT_CONFIG_DIR), |dir| dir.strip_prefix("/").unwrap_or(dir));
            self.home.join(config).join("agent")
        });
        agent.join("sessions")
    }

    /// Codex CLI's rollout files: `$CODEX_HOME/sessions`, else `~/.codex/sessions`.
    pub fn codex_sessions(&self) -> PathBuf {
        self.codex_home.join("sessions")
    }

    pub fn opencode_databases(&self) -> Vec<PathBuf> {
        if let Some(db) = &self.opencode_db {
            return vec![db.clone()];
        }
        let data = self.xdg_data_home.join(opencode::DATA_DIR);
        let mut found: Vec<PathBuf> = std::fs::read_dir(&data)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
                            name.starts_with("opencode") && name.to_ascii_lowercase().ends_with(".db")
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        if found.is_empty() {
            found.push(data.join(opencode::DB_FILE));
        }
        found
    }
}
