// Derived from plannotator-tui (https://github.com/plannotator/plannotator-tui) @ 83b0cd9, Copyright (c) 2026 Michael Ramos, MIT License. Changes for Annorex: kept only Claude Code, Pi, and OpenCode; removed sniff, detect_host, session_id_of, and is_uuid; added the roots module.
//! Find a coding agent's transcript on disk and read its recent rendered messages.
//!
//! The readers are pure over strings, slices, and injected directories: no `~`, no
//! environment reads, no spawned processes. The caller passes in `sessions_dir`,
//! `projects_dir`, a process-table snapshot, and an env lookup. Only [`roots`] reads the
//! process environment, and only through [`roots::Roots::from_env`].

pub mod changes;
pub mod claude;
pub mod opencode;
pub mod opencode_changes;
pub mod pi;
pub mod pi_changes;
pub mod roots;
pub(crate) mod sqlite;
pub(crate) mod time;

use std::path::PathBuf;

/// A supported agent host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    ClaudeCode,
    Pi,
    /// `OpenCode`: sessions, messages and parts in `SQLite` (`<xdg data>/opencode/opencode.db`),
    /// addressed by session id or found by the directory `OpenCode` was started in.
    OpenCode,
}

impl Host {
    /// The short label the UI uses.
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Pi => "pi",
            Self::OpenCode => "opencode",
        }
    }

    /// Every host with a transcript reader, for messages that list them.
    pub const ALL: [Host; 3] = [Host::ClaudeCode, Host::Pi, Host::OpenCode];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Human,
    Assistant,
}

/// One rendered message: every text block that shares the host's message id, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub text: String,
    /// The host's timestamp, verbatim, when it has one.
    pub at: Option<String>,
}

/// Newest-first messages up to and including the `n`th assistant reply. `n` counts replies,
/// never prompts: `last` offers replies, so a prompt still waiting for its answer must not
/// crowd out the reply before it. Prompts newer than that reply are kept for context.
pub(crate) fn through_replies(newest_first: impl IntoIterator<Item = Message>, n: usize) -> Vec<Message> {
    let mut out = Vec::new();
    let mut replies = 0;
    for message in newest_first {
        if replies >= n {
            break;
        }
        replies += usize::from(message.role == Role::Assistant);
        out.push(message);
    }
    out
}

/// Does `messages` hold an assistant reply? A transcript with prompts only (a conversation
/// just started) has nothing to annotate, so discovery passes over it.
pub(crate) fn has_reply(messages: &[Message]) -> bool {
    messages.iter().any(|message| message.role == Role::Assistant)
}

/// Which rung of a discovery ladder chose a session, so a caller can say whether the
/// result identifies the agent's own session or is the best guess for its folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// The session the agent process (or one of its ancestors) registered: not a guess.
    Session,
    /// A session whose recorded working directory is the agent's, newest first.
    Cwd,
    /// The newest session filed under the agent's folder or one of its parents.
    Folder,
    /// The newest session the host has at all, not scoped to any directory.
    Newest,
}

/// `~/.claude/sessions/<pid>.json`: one running Claude Code session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    pub pid: u32,
    pub session_id: String,
    pub cwd: PathBuf,
    pub started_at: u64,
}

#[derive(Debug)]
pub enum HostError {
    /// No transcript could be found; the message names what was searched.
    NoTranscript(String),
    /// A transcript was found but holds no renderable message.
    NoMessages(String),
    /// A host was recognised from the environment but is not supported yet.
    Unsupported(String),
    /// An exact session id is not one safe path component.
    InvalidSessionId(String),
    Io(std::io::Error),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTranscript(msg) | Self::NoMessages(msg) | Self::InvalidSessionId(msg) => f.write_str(msg),
            Self::Unsupported(host) => write!(f, "{host} is not supported yet"),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

/// Reject an exact session id before a resolver touches the filesystem or a database.
pub fn validate_session_id(id: &str) -> Result<&str, HostError> {
    if id.trim().is_empty() || id.contains(['/', '\\', '\0']) || id.contains("..") {
        return Err(HostError::InvalidSessionId(format!(
            "invalid session id {id:?}: expected one non-empty path component"
        )));
    }
    Ok(id)
}

impl std::error::Error for HostError {}

impl From<std::io::Error> for HostError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}
