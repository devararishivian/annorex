//! Drafts: the review of one document, saved to disk after each change.
//!
//! A draft is never deleted by Annorex on its own. A file that Annorex cannot read is kept as it
//! is and never written over, because it can hold the work of the user.

use std::ffi::OsString;
use std::io::Write;
use std::ops::Range;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agent::Agent;
use crate::review::{Kind, Review};

/// The version of the draft file.
const VERSION: u32 = 1;
/// The longest part of a file name, in characters.
const PART_LIMIT: usize = 80;

/// A problem with a draft file.
#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    /// The file exists but Annorex cannot use it. It stays as it is.
    #[error("the draft {} cannot be read; it was kept as it is", path.display())]
    Unreadable { path: PathBuf },
    #[error("cannot write the draft {}: {source}", path.display())]
    Io { path: PathBuf, source: std::io::Error },
}

/// Which review a draft belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftKey {
    pub agent: Agent,
    /// The agent session (`SessionRef::id`).
    pub session: String,
    /// The message or the file.
    pub document: String,
}

/// The editor that was open when the draft was saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Editing {
    pub kind: Kind,
    pub range: Range<usize>,
    pub text: String,
    pub label: Option<String>,
    /// True for the editor of the general note, which is not an annotation.
    #[serde(default)]
    pub general: bool,
}

/// The review of one document and its state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub key: DraftKey,
    /// What the user sees, for example `last reply` or a path.
    pub name: String,
    pub review: Review,
    pub editing: Option<Editing>,
    /// RFC 3339, UTC.
    pub updated_at: String,
    /// RFC 3339, UTC. Set after a send or a paste. A new change clears it.
    pub sent_at: Option<String>,
}

/// The time now as RFC 3339 in UTC.
pub fn now() -> String {
    jiff::Timestamp::now().to_string()
}

impl Draft {
    /// An empty draft.
    pub fn new(key: DraftKey, name: &str) -> Self {
        Self {
            key,
            name: name.to_owned(),
            review: Review::default(),
            editing: None,
            updated_at: now(),
            sent_at: None,
        }
    }

    /// Record a change: set the time and make the draft unsent again.
    pub fn touch(&mut self) {
        self.updated_at = now();
        self.sent_at = None;
    }

    /// Record a send or a paste.
    pub fn mark_sent(&mut self) {
        self.sent_at = Some(now());
    }
}

/// The file as it stands on disk.
#[derive(Deserialize)]
struct FileIn {
    version: u32,
    #[serde(flatten)]
    draft: Draft,
}

#[derive(Serialize)]
struct FileOut<'a> {
    version: u32,
    #[serde(flatten)]
    draft: &'a Draft,
}

/// The directory of the drafts: `$XDG_STATE_HOME/annorex/drafts` when that variable is an
/// absolute path, else `$HOME/.local/state/annorex/drafts`.
pub fn drafts_dir(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let absolute = |name: &str| env(name).map(PathBuf::from).filter(|path| path.is_absolute());
    let base =
        absolute("XDG_STATE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".local/state")))?;
    Some(base.join("annorex").join("drafts"))
}

/// A part of a file name: only `[A-Za-z0-9._-]`, at most 80 characters.
fn safe(part: &str) -> String {
    part.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .take(PART_LIMIT)
        .collect()
}

/// The draft files in one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftStore {
    pub dir: PathBuf,
}

impl DraftStore {
    /// The file of a draft.
    pub fn path(&self, key: &DraftKey) -> PathBuf {
        let name = format!("{}-{}-{}.json", key.agent.status_app(), safe(&key.session), safe(&key.document));
        self.dir.join(name)
    }

    fn read(path: &Path) -> Result<Option<Draft>, DraftError> {
        let unreadable = || DraftError::Unreadable { path: path.to_path_buf() };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(unreadable()),
        };
        let file: FileIn = serde_json::from_slice(&bytes).map_err(|_| unreadable())?;
        if file.version == VERSION { Ok(Some(file.draft)) } else { Err(unreadable()) }
    }

    /// The saved draft of `key`. `None` when there is no file. A file that cannot be read, has
    /// another version, or holds another key is `Unreadable`.
    ///
    /// The annotations are not checked against the document here. Call `Review::reanchor`.
    pub fn load(&self, key: &DraftKey) -> Result<Option<Draft>, DraftError> {
        let path = self.path(key);
        match Self::read(&path)? {
            Some(draft) if draft.key == *key => Ok(Some(draft)),
            Some(_) => Err(DraftError::Unreadable { path }),
            None => Ok(None),
        }
    }

    /// Write the draft to `<file>.tmp`, then rename it over the file. Refuses to write over a
    /// file that `load` cannot read.
    pub fn save(&self, draft: &Draft) -> Result<(), DraftError> {
        self.load(&draft.key)?;
        let path = self.path(&draft.key);
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| DraftError::Io { path, source }
        };
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&self.dir).map_err(io(&self.dir))?;
        let bytes = serde_json::to_vec_pretty(&FileOut { version: VERSION, draft })
            .map_err(|error| DraftError::Io { path: path.clone(), source: error.into() })?;
        let tmp = path.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(io(&tmp))?;
        file.write_all(&bytes).and_then(|()| file.sync_all()).map_err(io(&tmp))?;
        std::fs::rename(&tmp, &path).map_err(io(&path))
    }

    /// Remove the file of a draft. A missing file is fine. This also removes a file that cannot
    /// be read: it is the way out of that state.
    pub fn discard(&self, key: &DraftKey) -> Result<(), DraftError> {
        let path = self.path(key);
        match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(DraftError::Io { path, source: error })
            }
            _ => Ok(()),
        }
    }

    /// The documents of one session that have annotations and no send, sorted.
    pub fn unsent(&self, agent: Agent, session: &str) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut out: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| Self::read(&entry.path()).ok().flatten())
            .filter(|draft| {
                draft.key.agent == agent
                    && draft.key.session == session
                    && !draft.review.is_empty()
                    && draft.sent_at.is_none()
            })
            .map(|draft| draft.key.document)
            .collect();
        out.sort();
        out
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::*;
    use crate::review::Kind;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| pairs.iter().find(|(k, _)| *k == key).map(|(_, value)| OsString::from(value))
    }

    fn key(document: &str) -> DraftKey {
        DraftKey { agent: Agent::ClaudeCode, session: "s1".to_owned(), document: document.to_owned() }
    }

    fn draft_with_comment(document: &str, source: &str) -> Draft {
        let mut draft = Draft::new(key(document), "last reply");
        let id = draft.review.add(source, Kind::Comment, 6..10);
        draft.review.update(id, |a| a.note = "Why?".to_owned());
        draft
    }

    fn store() -> (tempfile::TempDir, DraftStore) {
        let dir = tempfile::tempdir().expect("dir");
        let store = DraftStore { dir: dir.path().join("drafts") };
        (dir, store)
    }

    #[test]
    fn save_then_load_gives_the_same_draft() {
        let (_dir, store) = store();
        let mut draft = draft_with_comment("r1", "alpha beta gamma\n");
        draft.review.note = "general".to_owned();
        draft.sent_at = Some("2026-11-09T10:00:00Z".to_owned());
        draft.editing = Some(Editing {
            kind: Kind::Replace,
            range: 0..5,
            text: "typed so far".to_owned(),
            label: Some("must fix".to_owned()),
            general: false,
        });
        store.save(&draft).expect("save");
        assert_eq!(store.load(&draft.key).expect("load"), Some(draft));
        assert_eq!(store.load(&key("other")).expect("load"), None);
    }

    #[test]
    fn the_file_has_a_version_and_a_readable_agent() {
        let (_dir, store) = store();
        let draft = draft_with_comment("r1", "alpha beta gamma\n");
        store.save(&draft).expect("save");
        let text = std::fs::read_to_string(store.path(&draft.key)).expect("file");
        let json: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(json["version"], 1);
        assert_eq!(json["key"]["agent"], "claude-code");
    }

    #[test]
    fn file_name_is_safe() {
        let key = DraftKey {
            agent: Agent::OpenCode,
            session: "a/b".to_owned(),
            document: "my file name.md".to_owned(),
        };
        let (_dir, store) = store();
        let name = store.path(&key).file_name().expect("name").to_string_lossy().into_owned();
        assert_eq!(name, "opencode-a_b-my_file_name.md.json");
        let long = DraftKey { document: "x".repeat(200), ..key };
        let name = store.path(&long).file_name().expect("name").to_string_lossy().into_owned();
        assert_eq!(name, format!("opencode-a_b-{}.json", "x".repeat(80)));
        assert!(store.path(&long).starts_with(&store.dir));
    }

    #[test]
    fn corrupt_draft_is_kept_and_reported() {
        let (_dir, store) = store();
        let draft = draft_with_comment("r1", "alpha beta gamma\n");
        store.save(&draft).expect("save");
        let path = store.path(&draft.key);
        std::fs::write(&path, "{bad").expect("corrupt");
        match store.load(&draft.key) {
            Err(DraftError::Unreadable { path: reported }) => assert_eq!(reported, path),
            other => panic!("{other:?}"),
        }
        assert!(matches!(store.save(&draft), Err(DraftError::Unreadable { .. })));
        assert_eq!(std::fs::read(&path).expect("bytes"), b"{bad");
        store.discard(&draft.key).expect("discard");
        assert!(!path.exists());
        store.save(&draft).expect("save after discard");
    }

    #[test]
    fn another_version_or_key_is_unreadable() {
        let (_dir, store) = store();
        let draft = draft_with_comment("r1", "alpha beta gamma\n");
        store.save(&draft).expect("save");
        let path = store.path(&draft.key);
        let text = std::fs::read_to_string(&path).expect("file").replace("\"version\": 1", "\"version\": 2");
        std::fs::write(&path, text).expect("write");
        assert!(matches!(store.load(&draft.key), Err(DraftError::Unreadable { .. })));
        store.discard(&draft.key).expect("discard");
        let other = draft_with_comment("r2", "alpha beta gamma\n");
        store.save(&other).expect("save");
        std::fs::copy(store.path(&other.key), store.path(&draft.key)).expect("copy");
        assert!(matches!(store.load(&draft.key), Err(DraftError::Unreadable { .. })));
    }

    #[test]
    fn save_is_atomic() {
        let (_dir, store) = store();
        let draft = draft_with_comment("r1", "alpha beta gamma\n");
        store.save(&draft).expect("save");
        store.save(&draft).expect("save again");
        let names: Vec<String> = std::fs::read_dir(&store.dir)
            .expect("dir")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["claude-code-s1-r1.json"]);
    }

    #[test]
    fn the_directory_is_private() {
        let (_dir, store) = store();
        store.save(&draft_with_comment("r1", "alpha beta gamma\n")).expect("save");
        let mode = |path: &Path| std::fs::metadata(path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode(&store.dir), 0o700);
        assert_eq!(mode(&store.path(&key("r1"))), 0o600);
    }

    #[test]
    fn discard_of_a_missing_draft_is_fine() {
        let (_dir, store) = store();
        store.discard(&key("none")).expect("discard");
    }

    #[test]
    fn unsent_lists_only_unsent_drafts() {
        let (_dir, store) = store();
        let source = "alpha beta gamma\n";
        store.save(&draft_with_comment("unsent", source)).expect("save");
        let mut sent = draft_with_comment("sent", source);
        sent.sent_at = Some("2026-11-09T10:00:00Z".to_owned());
        store.save(&sent).expect("save");
        store.save(&Draft::new(key("empty"), "x")).expect("save");
        let mut other_session = draft_with_comment("other", source);
        other_session.key.session = "s1-b".to_owned();
        store.save(&other_session).expect("save");
        let mut other_agent = draft_with_comment("agent", source);
        other_agent.key.agent = Agent::Pi;
        store.save(&other_agent).expect("save");
        std::fs::write(store.dir.join("claude-code-s1-broken.json"), "{bad").expect("broken");
        assert_eq!(store.unsent(Agent::ClaudeCode, "s1"), ["unsent"]);
        assert_eq!(
            DraftStore { dir: PathBuf::from("/nonexistent/x") }.unsent(Agent::Pi, "s"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_change_makes_a_sent_draft_unsent() {
        let mut draft = draft_with_comment("r1", "alpha beta gamma\n");
        draft.mark_sent();
        assert!(draft.sent_at.is_some());
        draft.touch();
        assert!(draft.sent_at.is_none());
        assert!(draft.updated_at.ends_with('Z'), "{}", draft.updated_at);
    }

    #[test]
    fn moved_quote_is_found_again() {
        let mut draft = draft_with_comment("r1", "alpha beta gamma\n");
        assert_eq!(draft.review.annotations[0].quote, "beta");
        draft.review.reanchor("new first line\nalpha beta gamma\n");
        let a = &draft.review.annotations[0];
        assert_eq!(a.range, 21..25);
        assert!(!a.detached);
    }

    #[test]
    fn missing_quote_is_detached() {
        let mut draft = draft_with_comment("r1", "alpha beta gamma\n");
        draft.review.reanchor("completely different\n");
        let a = &draft.review.annotations[0];
        assert!(a.detached);
        assert_eq!(a.range, 6..10);
        draft.review.reanchor("alpha beta gamma\n");
        assert!(!draft.review.annotations[0].detached, "a quote that comes back is attached again");
    }

    #[test]
    fn a_quote_that_still_fits_stays_put() {
        let mut draft = draft_with_comment("r1", "alpha beta gamma\n");
        draft.review.reanchor("alpha beta gamma\n");
        assert_eq!(draft.review.annotations[0].range, 6..10);
    }

    #[test]
    fn drafts_dir_uses_xdg_state_home() {
        let dir = drafts_dir(env(&[("XDG_STATE_HOME", "/x"), ("HOME", "/h")]));
        assert_eq!(dir, Some(PathBuf::from("/x/annorex/drafts")));
        let home = drafts_dir(env(&[("HOME", "/h")]));
        assert_eq!(home, Some(PathBuf::from("/h/.local/state/annorex/drafts")));
        assert_eq!(drafts_dir(env(&[])), None);
    }

    #[test]
    fn drafts_dir_ignores_a_relative_value() {
        let dir = drafts_dir(env(&[("XDG_STATE_HOME", "rel"), ("HOME", "/h")]));
        assert_eq!(dir, Some(PathBuf::from("/h/.local/state/annorex/drafts")));
    }
}
