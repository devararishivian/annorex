//! The check for a newer release: a daily cache and one request to the GitHub release API.

use std::ffi::OsString;
use std::path::Path;
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
    let temporary = std::path::PathBuf::from(temporary);
    std::fs::write(&temporary, serde_json::to_vec(cache)?)?;
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

/// True when no check happened in the last `CHECK_INTERVAL_SECS`.
pub fn due(cache: Option<&Cache>, now: u64) -> bool {
    cache.is_none_or(|cache| now.saturating_sub(cache.checked_at) >= CHECK_INTERVAL_SECS)
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
}
