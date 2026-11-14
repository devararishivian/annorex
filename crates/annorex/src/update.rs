//! The check for a newer release: a daily cache and one request to the GitHub release API.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

/// The time between two checks, in seconds.
pub const CHECK_INTERVAL_SECS: u64 = 86_400;

const ENDPOINT: &str = "https://api.github.com/repos/devararishivian/annorex/releases/latest";

/// The result of the last check, kept in the file `update-check.json` in the state directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    /// The time of the last check, in seconds since the Unix epoch.
    pub checked_at: u64,
    /// The newest version that a check found, without a leading `v`.
    pub latest: Option<String>,
}

/// Reads the cache. A missing, empty, or broken file gives `None`.
pub fn read_cache(path: &Path) -> Option<Cache> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Writes the cache to a temporary file, then renames it over the real file.
pub fn write_cache(path: &Path, cache: &Cache) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, serde_json::to_vec(cache)?)?;
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

/// True when no check happened in the last `CHECK_INTERVAL_SECS`, or when the time of the last
/// check is in the future (the clock was set back).
pub fn due(cache: Option<&Cache>, now: u64) -> bool {
    cache.is_none_or(|cache| cache.checked_at > now || now - cache.checked_at >= CHECK_INTERVAL_SECS)
}

/// The version in the `tag_name` field of a release, without a leading `v`.
pub fn parse_latest(json: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
    }
    let release: Release = serde_json::from_str(json).ok()?;
    Some(release.tag_name.strip_prefix('v').unwrap_or(&release.tag_name).to_owned())
}

/// The numbers of a version of the form `X.Y.Z`.
fn parts(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.split('.');
    let found = [parts.next()?, parts.next()?, parts.next()?];
    if parts.next().is_some() {
        return None;
    }
    let mut numbers = [0; 3];
    for (number, text) in numbers.iter_mut().zip(found) {
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *number = text.parse().ok()?;
    }
    Some(numbers)
}

/// True only when both are `X.Y.Z` with numbers and `latest` is greater than `current`.
pub fn newer(current: &str, latest: &str) -> bool {
    match (parts(current), parts(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

/// True when the build or the environment turns the update check off.
pub fn disabled(env: impl Fn(&str) -> Option<OsString>) -> bool {
    option_env!("ANNOREX_DISABLE_UPDATE_CHECK").is_some()
        || env("ANNOREX_NO_UPDATE_CHECK").is_some_and(|value| !value.is_empty() && value != "0")
}

/// Asks GitHub for the latest release. Any failure gives `None`.
pub fn fetch_latest() -> Option<String> {
    let output = Command::new("/usr/bin/curl")
        .args(["-fsS", "--max-time", "5"])
        .args(["-H", "Accept: application/vnd.github+json"])
        .args(["-H", "X-GitHub-Api-Version: 2026-03-10"])
        .args(["-H", concat!("User-Agent: annorex/", env!("CARGO_PKG_VERSION"))])
        .arg(ENDPOINT)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_latest(std::str::from_utf8(&output.stdout).ok()?)
}

/// The latest version to compare. When a check is due, this calls `fetch` and writes the cache.
/// A failed fetch keeps the old `latest` and still records the time of the check.
pub fn cached_latest(path: &Path, now: u64, fetch: impl FnOnce() -> Option<String>) -> Option<String> {
    let cache = read_cache(path);
    if !due(cache.as_ref(), now) {
        return cache.and_then(|cache| cache.latest);
    }
    let latest = fetch().or_else(|| cache.and_then(|cache| cache.latest));
    let _ = write_cache(path, &Cache { checked_at: now, latest: latest.clone() });
    latest
}

/// The file name of the cache in the state directory.
const CACHE_FILE: &str = "update-check.json";

/// The path of the cache file.
pub fn cache_path(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    Some(crate::draft::state_dir(env)?.join(CACHE_FILE))
}

/// How the user installed Annorex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// The Homebrew formula.
    Homebrew,
    /// The shell installer of the release.
    Installer,
    /// `cargo install`, or any other way.
    Cargo,
}

#[derive(Deserialize)]
struct Receipt {
    install_prefix: String,
}

/// The install prefix in the receipt of the shell installer, field `install_prefix`. The receipt is
/// `$XDG_CONFIG_HOME/annorex/annorex-receipt.json`, else `$HOME/.config/annorex/annorex-receipt.json`.
/// A missing or broken receipt gives `None`.
pub fn receipt_prefix(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let absolute = |name: &str| env(name).map(PathBuf::from).filter(|path| path.is_absolute());
    let base = absolute("XDG_CONFIG_HOME").or_else(|| absolute("HOME").map(|home| home.join(".config")))?;
    let receipt: Receipt =
        serde_json::from_slice(&std::fs::read(base.join("annorex").join("annorex-receipt.json")).ok()?)
            .ok()?;
    (!receipt.install_prefix.is_empty()).then(|| PathBuf::from(receipt.install_prefix))
}

/// The canonical form of a path, or the path itself when it cannot be resolved.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The install channel of the binary `exe`.
pub fn channel(exe: &Path, receipt_prefix: Option<&Path>) -> Channel {
    let exe = canonical(exe);
    let mut names = exe.components().map(std::path::Component::as_os_str);
    let in_cellar = names.any(|name| name == "Cellar") && names.next().is_some_and(|name| name == "annorex");
    if in_cellar {
        return Channel::Homebrew;
    }
    match receipt_prefix {
        Some(prefix) if exe.starts_with(canonical(prefix)) => Channel::Installer,
        _ => Channel::Cargo,
    }
}

const INSTALLER_COMMAND: &str = "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/devararishivian/annorex/releases/latest/download/annorex-installer.sh | sh";

/// The command that upgrades Annorex, or `None` when the user must upgrade by hand.
pub fn upgrade_command(channel: Channel) -> Option<Vec<String>> {
    let words: &[&str] = match channel {
        Channel::Homebrew => &["brew", "upgrade", "annorex"],
        Channel::Installer => &["sh", "-c", INSTALLER_COMMAND],
        Channel::Cargo => return None,
    };
    Some(words.iter().map(|word| (*word).to_owned()).collect())
}

/// The command to show to the user.
pub fn command_text(channel: Channel) -> &'static str {
    match channel {
        Channel::Homebrew => "brew upgrade annorex",
        Channel::Installer => INSTALLER_COMMAND,
        Channel::Cargo => "cargo install --git https://github.com/devararishivian/annorex --locked annorex",
    }
}

/// The short hint for a notice in the viewer.
pub fn hint(channel: Channel) -> &'static str {
    match channel {
        Channel::Homebrew => "brew upgrade annorex",
        Channel::Installer | Channel::Cargo => "annorex update",
    }
}

/// What `annorex update` does after it knows the latest version.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// The current version is the newest.
    Current,
    /// Print the line and stop.
    Notice(String),
    /// Print `Running: ...` and run the command.
    Run(Vec<String>),
}

/// Decides the action of `annorex update`. It runs nothing.
pub fn plan(current: &str, latest: &str, check_only: bool, channel: Channel) -> Plan {
    if !newer(current, latest) {
        return Plan::Current;
    }
    let notice = || Plan::Notice(format!("Annorex {latest} is available. Run: {}", command_text(channel)));
    match upgrade_command(channel) {
        Some(command) if !check_only => Plan::Run(command),
        _ => notice(),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| pairs.iter().find(|(k, _)| *k == key).map(|(_, value)| OsString::from(value))
    }

    #[test]
    fn newer_compares_numbers() {
        assert!(newer("0.1.0", "0.2.0"));
        assert!(!newer("0.2.0", "0.2.0"));
        assert!(!newer("0.10.0", "0.9.9"));
        assert!(newer("0.9.9", "0.10.0"));
    }

    #[test]
    fn newer_ignores_a_tag_that_is_not_a_version() {
        assert!(!newer("0.1.0", "0.2.0-rc.1"));
        assert!(!newer("0.1.0", "nightly"));
        assert!(!newer("0.1.0", "0.2"));
        assert!(!newer("nightly", "0.2.0"));
    }

    #[test]
    fn parse_latest_strips_the_v() {
        assert_eq!(parse_latest(r#"{"tag_name":"v0.2.0"}"#), Some("0.2.0".to_owned()));
        assert_eq!(parse_latest(r#"{"tag_name":"0.3.0"}"#), Some("0.3.0".to_owned()));
        assert_eq!(parse_latest("{"), None);
        assert_eq!(parse_latest("{}"), None);
    }

    #[test]
    fn due_after_one_day() {
        assert!(due(None, 100));
        let cache = Cache { checked_at: 100, latest: None };
        assert!(!due(Some(&cache), 100 + 86_399));
        assert!(due(Some(&cache), 100 + 86_400));
    }

    #[test]
    fn future_checked_at_is_due() {
        let cache = Cache { checked_at: 1_000, latest: None };
        assert!(due(Some(&cache), 999));
        assert!(!due(Some(&cache), 1_000));
    }

    #[test]
    fn broken_cache_counts_as_no_cache() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("update-check.json");
        assert_eq!(read_cache(&path), None);
        std::fs::write(&path, "{").expect("write");
        assert_eq!(read_cache(&path), None);
        std::fs::write(&path, "").expect("write");
        assert_eq!(read_cache(&path), None);
    }

    #[test]
    fn cache_round_trips() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("sub").join("update-check.json");
        let cache = Cache { checked_at: 5, latest: Some("0.2.0".to_owned()) };
        write_cache(&path, &cache).expect("write");
        assert_eq!(read_cache(&path), Some(cache));
    }

    #[test]
    fn failed_check_is_cached_and_keeps_the_old_latest() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("update-check.json");
        write_cache(&path, &Cache { checked_at: 0, latest: Some("0.2.0".to_owned()) }).expect("write");
        let latest = cached_latest(&path, 90_000, || None);
        assert_eq!(latest, Some("0.2.0".to_owned()));
        assert_eq!(read_cache(&path).expect("cache").checked_at, 90_000);
        let again = cached_latest(&path, 90_001, || panic!("must not fetch"));
        assert_eq!(again, Some("0.2.0".to_owned()));
    }

    #[test]
    fn a_new_fetch_replaces_the_latest() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("update-check.json");
        let latest = cached_latest(&path, 10, || Some("0.3.0".to_owned()));
        assert_eq!(latest, Some("0.3.0".to_owned()));
        assert_eq!(read_cache(&path), Some(Cache { checked_at: 10, latest: Some("0.3.0".to_owned()) }));
    }

    #[test]
    fn disabled_reads_the_variable() {
        assert!(disabled(env(&[("ANNOREX_NO_UPDATE_CHECK", "1")])));
        assert!(!disabled(env(&[("ANNOREX_NO_UPDATE_CHECK", "0")])));
        assert!(!disabled(env(&[("ANNOREX_NO_UPDATE_CHECK", "")])));
        assert!(!disabled(env(&[])));
    }

    #[test]
    fn channel_finds_homebrew_by_the_cellar_path() {
        let exe = Path::new("/opt/homebrew/Cellar/annorex/0.2.0/bin/annorex");
        assert_eq!(channel(exe, None), Channel::Homebrew);
    }

    #[test]
    fn channel_finds_the_installer_by_the_receipt_prefix() {
        let exe = Path::new("/u/.cargo/bin/annorex");
        assert_eq!(channel(exe, Some(Path::new("/u/.cargo"))), Channel::Installer);
        assert_eq!(channel(exe, None), Channel::Cargo);
        assert_eq!(channel(exe, Some(Path::new("/other"))), Channel::Cargo);
    }

    #[test]
    fn channel_resolves_symlinks() {
        let dir = tempfile::tempdir().expect("dir");
        let real = dir.path().join("Cellar/annorex/0.2.0/bin");
        std::fs::create_dir_all(&real).expect("dirs");
        std::fs::write(real.join("annorex"), "").expect("file");
        std::fs::create_dir_all(dir.path().join("bin")).expect("bin");
        let link = dir.path().join("bin/annorex");
        std::os::unix::fs::symlink(real.join("annorex"), &link).expect("link");
        assert_eq!(channel(&link, None), Channel::Homebrew);
        let prefix = dir.path().join("bin");
        assert_eq!(channel(&link, Some(&prefix)), Channel::Homebrew);
        let plain = dir.path().join("bin/other");
        std::fs::write(&plain, "").expect("file");
        assert_eq!(channel(&plain, Some(dir.path())), Channel::Installer);
    }

    #[test]
    fn receipt_prefix_reads_the_receipt() {
        let dir = tempfile::tempdir().expect("dir");
        let folder = dir.path().join("annorex");
        std::fs::create_dir_all(&folder).expect("dir");
        let xdg = dir.path().to_str().expect("utf8");
        let pairs = [("XDG_CONFIG_HOME", xdg)];
        let reader = env(&pairs);
        assert_eq!(receipt_prefix(&reader), None);
        let file = folder.join("annorex-receipt.json");
        std::fs::write(&file, "{").expect("write");
        assert_eq!(receipt_prefix(&reader), None);
        std::fs::write(&file, r#"{"install_prefix":"/u/.cargo","version":"0.1.0"}"#).expect("write");
        assert_eq!(receipt_prefix(&reader), Some(PathBuf::from("/u/.cargo")));
    }

    #[test]
    fn receipt_prefix_falls_back_to_home() {
        let dir = tempfile::tempdir().expect("dir");
        let folder = dir.path().join(".config/annorex");
        std::fs::create_dir_all(&folder).expect("dir");
        std::fs::write(folder.join("annorex-receipt.json"), r#"{"install_prefix":"/p"}"#).expect("write");
        let home = dir.path().to_str().expect("utf8");
        assert_eq!(receipt_prefix(env(&[("HOME", home)])), Some(PathBuf::from("/p")));
    }

    #[test]
    fn upgrade_commands_per_channel() {
        assert_eq!(upgrade_command(Channel::Cargo), None);
        assert_eq!(
            upgrade_command(Channel::Homebrew),
            Some(vec!["brew".into(), "upgrade".into(), "annorex".into()])
        );
        let installer = upgrade_command(Channel::Installer).expect("command");
        let [shell, flag, script] = installer.as_slice() else { panic!("three words") };
        assert_eq!((shell.as_str(), flag.as_str()), ("sh", "-c"));
        assert!(script.ends_with("annorex-installer.sh | sh"));
    }

    #[test]
    fn hints_per_channel() {
        assert_eq!(hint(Channel::Homebrew), "brew upgrade annorex");
        assert_eq!(hint(Channel::Installer), "annorex update");
        assert_eq!(hint(Channel::Cargo), "annorex update");
    }

    #[test]
    fn plan_decides_without_running() {
        assert_eq!(plan("0.2.0", "0.2.0", false, Channel::Homebrew), Plan::Current);
        assert_eq!(plan("0.2.0", "nightly", false, Channel::Homebrew), Plan::Current);
        assert_eq!(
            plan("0.1.0", "0.2.0", true, Channel::Homebrew),
            Plan::Notice("Annorex 0.2.0 is available. Run: brew upgrade annorex".to_owned())
        );
        assert_eq!(
            plan("0.1.0", "0.2.0", false, Channel::Homebrew),
            Plan::Run(upgrade_command(Channel::Homebrew).expect("command"))
        );
        let cargo = "Annorex 0.2.0 is available. Run: cargo install --git https://github.com/devararishivian/annorex --locked annorex";
        assert_eq!(plan("0.1.0", "0.2.0", false, Channel::Cargo), Plan::Notice(cargo.to_owned()));
        assert_eq!(plan("0.1.0", "0.2.0", true, Channel::Cargo), Plan::Notice(cargo.to_owned()));
    }

    #[test]
    fn cache_path_is_in_the_state_dir() {
        assert_eq!(
            cache_path(env(&[("XDG_STATE_HOME", "/x")])),
            Some(PathBuf::from("/x/annorex/update-check.json"))
        );
    }
}
