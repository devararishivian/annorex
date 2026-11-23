// Derived from plannotator-tui (https://github.com/plannotator/plannotator-tui) @ 83b0cd9, Copyright (c) 2026 Michael Ramos, MIT License. Changes for Annorex: moved from the unit tests of crates/plannotator-tui/src/last/roots.rs; removed the cases for other hosts; reads the roots through public accessors.
//! Storage roots honour the documented environment overrides.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use annorex_hosts::opencode;
use annorex_hosts::roots::Roots;

#[test]
fn every_documented_root_override_is_honoured() {
    let values: HashMap<&str, OsString> = [
        ("CLAUDE_CONFIG_DIR", "root/claude".into()),
        ("PI_CODING_AGENT_SESSION_DIR", "root/pi sessions".into()),
        ("PI_CODING_AGENT_DIR", "root/pi agent ignored".into()),
        ("OPENCODE_DB", "root/opencode/custom.db".into()),
        ("XDG_DATA_HOME", "root/xdg ignored".into()),
    ]
    .into_iter()
    .collect();
    let roots = Roots::from_lookup(PathBuf::from("home"), |key| values.get(key).cloned());
    assert_eq!(roots.claude_config(), Path::new("root/claude"));
    assert_eq!(roots.pi_sessions(), PathBuf::from("root/pi sessions"));
    assert_eq!(roots.opencode_databases(), vec![PathBuf::from("root/opencode/custom.db")]);
}

#[test]
fn pi_agent_and_xdg_overrides_apply_when_the_more_specific_values_are_absent() {
    let values: HashMap<&str, OsString> =
        [("PI_CODING_AGENT_DIR", "root/pi agent ü".into()), ("XDG_DATA_HOME", "root/data ü".into())]
            .into_iter()
            .collect();
    let roots = Roots::from_lookup(PathBuf::from("home"), |key| values.get(key).cloned());
    assert_eq!(roots.pi_sessions(), PathBuf::from("root/pi agent ü/sessions"));
    assert_eq!(
        roots.opencode_databases(),
        vec![PathBuf::from("root/data ü").join(opencode::DATA_DIR).join(opencode::DB_FILE)]
    );
}

#[test]
fn defaults_are_under_the_home_directory() {
    let roots = Roots::from_lookup(PathBuf::from("home"), |_| None);
    assert_eq!(roots.claude_config(), Path::new("home/.claude"));
    assert_eq!(roots.pi_sessions(), PathBuf::from("home/.pi/agent/sessions"));
}

#[test]
fn codex_sessions_follow_codex_home() {
    let set: HashMap<&str, OsString> = [("CODEX_HOME", "root/codex".into())].into_iter().collect();
    let roots = Roots::from_lookup(PathBuf::from("home"), |key| set.get(key).cloned());
    assert_eq!(roots.codex_sessions(), PathBuf::from("root/codex/sessions"));
    let empty: HashMap<&str, OsString> = [("CODEX_HOME", "".into())].into_iter().collect();
    let roots = Roots::from_lookup(PathBuf::from("home"), |key| empty.get(key).cloned());
    assert_eq!(roots.codex_sessions(), PathBuf::from("home/.codex/sessions"));
    let roots = Roots::from_lookup(PathBuf::from("home"), |_| None);
    assert_eq!(roots.codex_sessions(), PathBuf::from("home/.codex/sessions"));
}

// OMP roots follow can1357/oh-my-pi @ v19.1.1 `packages/utils/src/dirs.ts`: `PI_CODING_AGENT_DIR` replaces
// `<root>/agent`; `PI_CONFIG_DIR` is a folder name under home (default `.omp`).
#[test]
fn omp_sessions_follow_the_agent_dir_then_the_config_dir_then_the_default() {
    let roots = |pairs: &[(&'static str, &'static str)]| {
        let set: HashMap<&str, OsString> = pairs.iter().map(|(k, v)| (*k, (*v).into())).collect();
        Roots::from_lookup(PathBuf::from("/h"), move |key| set.get(key).cloned())
    };
    assert_eq!(roots(&[]).omp_sessions(), PathBuf::from("/h/.omp/agent/sessions"));
    assert_eq!(roots(&[("PI_CONFIG_DIR", ".cfg")]).omp_sessions(), PathBuf::from("/h/.cfg/agent/sessions"));
    assert_eq!(roots(&[("PI_CONFIG_DIR", "")]).omp_sessions(), PathBuf::from("/h/.omp/agent/sessions"));
    assert_eq!(
        roots(&[("PI_CONFIG_DIR", ".cfg"), ("PI_CODING_AGENT_DIR", "/x/agent")]).omp_sessions(),
        PathBuf::from("/x/agent/sessions")
    );
    // The Pi folder variable for sessions is not an OMP root.
    assert_eq!(
        roots(&[("PI_CODING_AGENT_SESSION_DIR", "/p")]).omp_sessions(),
        PathBuf::from("/h/.omp/agent/sessions")
    );
}
