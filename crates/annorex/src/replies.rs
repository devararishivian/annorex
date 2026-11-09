//! The replies of an agent: find its session, check whether the session is ambiguous, and read
//! the newest replies.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use annorex_hosts::opencode::Found;
use annorex_hosts::roots::Roots;
use annorex_hosts::{HostError, Message, Role, claude, opencode, pi};

use crate::agent::Agent;
use crate::rex::{ProcessInfo, Rex, RexError, Runner};

/// The reply list shows a maximum of this many replies (spec §6.3).
pub const REPLY_LIMIT: usize = 25;

/// Two sessions of one directory that changed in this time make the session ambiguous.
pub const RECENT: Duration = Duration::from_mins(10);

/// The creator of terminal blocks.
const TERMINAL_CREATOR: &str = "com.superlogical.terminal";

/// One reply of the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub id: String,
    pub text: String,
    /// The timestamp of the agent, as the agent wrote it.
    pub at: Option<String>,
}

impl Reply {
    /// The first line that is not blank, trimmed.
    pub fn first_line(&self) -> &str {
        first_line(&self.text)
    }
}

fn first_line(text: &str) -> &str {
    text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("")
}

/// Where the replies of one agent session are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRef {
    Claude(PathBuf),
    Pi(PathBuf),
    OpenCode { db: PathBuf, found: Found },
}

/// A session that can hold the replies of the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub session: SessionRef,
    pub updated: SystemTime,
    /// The first line of the newest reply.
    pub last_line: String,
}

/// The agent process in the target pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub agent: Agent,
    pub cwd: PathBuf,
    pub pid: u32,
}

impl Source {
    /// The foreground process of the pane. `None` when Rex does not know its directory.
    pub fn from_process(agent: Agent, process: &ProcessInfo) -> Option<Self> {
        let foreground = process.foreground.as_ref()?;
        Some(Self { agent, cwd: foreground.cwd.clone()?, pid: foreground.pid })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("no {} session for {} in {searched}", agent_name(*.agent), .cwd.display())]
    NoTranscript { agent: Agent, cwd: PathBuf, searched: String },
    #[error("the agent has no reply yet; press r to read the transcript again")]
    NoReply,
    #[error("cannot read the transcript: {0}")]
    Host(#[from] HostError),
    #[error("cannot read the transcript: {0}")]
    Io(#[from] std::io::Error),
}

fn agent_name(agent: Agent) -> &'static str {
    match agent {
        Agent::ClaudeCode => "Claude Code",
        Agent::Pi => "Pi",
        Agent::OpenCode => "OpenCode",
    }
}

/// The process table (`ps -eo pid=,ppid=`) that the Claude Code reader uses to find the session
/// of a process.
pub fn process_table() -> std::io::Result<Vec<(u32, u32)>> {
    let output = std::process::Command::new("ps").args(["-eo", "pid=,ppid="]).output()?;
    Ok(claude::parse_ps(&String::from_utf8_lossy(&output.stdout)))
}

/// The sessions that can hold the replies of `source`. Claude Code gives one session (found by
/// pid). Pi and `OpenCode` give all sessions of the directory, newest first.
pub fn candidates(
    source: &Source,
    roots: &Roots,
    process_table: &[(u32, u32)],
) -> Result<Vec<Candidate>, LoadError> {
    let mut found = Vec::new();
    let searched;
    match source.agent {
        Agent::ClaudeCode => {
            let config = roots.claude_config();
            searched = config.display().to_string();
            if let Some((path, _)) = claude::find_transcript(
                &config.join("sessions"),
                &config.join("projects"),
                &source.cwd,
                process_table,
                source.pid,
            ) {
                let updated = modified(&path);
                found.push((SessionRef::Claude(path), updated));
            }
        }
        Agent::Pi => {
            let sessions = roots.pi_sessions();
            searched = sessions.display().to_string();
            for (path, updated) in pi::sessions_for_cwd(&sessions, &source.cwd) {
                found.push((SessionRef::Pi(path), updated));
            }
        }
        Agent::OpenCode => {
            let databases = roots.opencode_databases();
            searched = databases.iter().map(|db| db.display().to_string()).collect::<Vec<_>>().join(", ");
            for db in databases {
                for session in opencode::sessions_for_cwd(&db, &source.cwd)? {
                    let updated =
                        UNIX_EPOCH + Duration::from_millis(u64::try_from(session.updated).unwrap_or(0));
                    found.push((SessionRef::OpenCode { db: db.clone(), found: session }, updated));
                }
            }
            found.sort_by(|(_, a), (_, b)| b.cmp(a));
        }
    }
    if found.is_empty() {
        return Err(LoadError::NoTranscript { agent: source.agent, cwd: source.cwd.clone(), searched });
    }
    Ok(found
        .into_iter()
        .map(|(session, updated)| {
            let last_line = load_replies(&session)
                .ok()
                .and_then(|replies| replies.first().map(|reply| reply.first_line().to_owned()))
                .unwrap_or_default();
            Candidate { session, updated, last_line }
        })
        .collect())
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path).and_then(|meta| meta.modified()).unwrap_or(UNIX_EPOCH)
}

/// True when Annorex must ask which session to show (spec §6.3). Claude Code is never ambiguous:
/// the reader finds it by pid.
pub fn needs_picker(agent: Agent, matching_panes: usize, candidates: &[Candidate], now: SystemTime) -> bool {
    if agent == Agent::ClaudeCode {
        return false;
    }
    let recent = candidates
        .iter()
        .filter(|candidate| now.duration_since(candidate.updated).is_ok_and(|age| age <= RECENT))
        .count();
    matching_panes > 1 || recent > 1
}

/// The number of terminal panes, in all sessions, that run `agent` in `cwd`. A pane that fails
/// the `process` call does not count.
pub fn matching_panes<R: Runner>(rex: &Rex<R>, agent: Agent, cwd: &Path) -> Result<usize, RexError> {
    let mut count = 0;
    for session in rex.sessions()? {
        let rex = rex.with_session(&session);
        let Ok(view) = rex.view() else { continue };
        for block in view.blocks().filter(|block| block.creator_name.as_deref() == Some(TERMINAL_CREATOR)) {
            let Ok(process) = rex.process(&block.block_id) else { continue };
            let Some(foreground) = process.foreground else { continue };
            let same_agent = foreground.argv0.as_deref() == Some(agent.argv0());
            if same_agent && foreground.cwd.as_deref().is_some_and(|dir| same_dir(dir, cwd)) {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a.components().eq(b.components())
}

/// The replies of `session`, newest first, a maximum of `REPLY_LIMIT`.
pub fn load_replies(session: &SessionRef) -> Result<Vec<Reply>, LoadError> {
    let messages: Vec<Message> = match session {
        SessionRef::Claude(path) => claude::parse_messages(&std::fs::read_to_string(path)?, REPLY_LIMIT),
        SessionRef::Pi(path) => pi::parse_messages(&std::fs::read_to_string(path)?, REPLY_LIMIT),
        SessionRef::OpenCode { db, found } => {
            opencode::messages_for_session(db, &found.id, found.schema, REPLY_LIMIT)?
        }
    };
    let replies: Vec<Reply> = messages
        .into_iter()
        .filter(|message| message.role == Role::Assistant)
        .take(REPLY_LIMIT)
        .map(|message| Reply { id: message.id, text: message.text, at: message.at })
        .collect();
    if replies.is_empty() {
        return Err(LoadError::NoReply);
    }
    Ok(replies)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::fmt::Write as _;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    use super::*;

    fn candidate(updated: SystemTime) -> Candidate {
        Candidate { session: SessionRef::Pi(PathBuf::from("/s.jsonl")), updated, last_line: String::new() }
    }

    /// A Claude Code transcript with `n` prompts and `n` replies, in one chain.
    fn claude_transcript(n: usize) -> String {
        let mut text = String::new();
        let mut parent = String::from("null");
        for i in 1..=n {
            let user = format!("u{i}");
            let assistant = format!("a{i}");
            writeln!(
                text,
                r#"{{"type":"user","uuid":"{user}","parentUuid":{parent},"message":{{"role":"user","content":"question {i}"}},"timestamp":"2026-11-09T10:{i:02}:00.000Z"}}"#
            )
            .expect("write");
            writeln!(
                text,
                r#"{{"type":"assistant","uuid":"{assistant}","parentUuid":"{user}","message":{{"id":"m{i}","role":"assistant","content":[{{"type":"text","text":"\n\nreply {i}\nmore"}}]}},"timestamp":"2026-11-09T10:{i:02}:30.000Z"}}"#
            )
            .expect("write");
            parent = format!("\"{assistant}\"");
        }
        text
    }

    #[test]
    fn claude_has_one_candidate_and_never_needs_picker() {
        let home = tempfile::tempdir().expect("home");
        let cwd = PathBuf::from("/Users/dev/app");
        let project = home.path().join(".claude/projects").join(claude::project_slug(&cwd));
        std::fs::create_dir_all(&project).expect("project");
        std::fs::write(project.join("s1.jsonl"), claude_transcript(2)).expect("transcript");
        let roots = Roots::from_lookup(home.path().to_path_buf(), |_| None);
        let source = Source { agent: Agent::ClaudeCode, cwd, pid: 4242 };
        let found = candidates(&source, &roots, &[]).expect("candidates");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].last_line, "reply 2");
        let now = SystemTime::now();
        let two = [candidate(now), candidate(now)];
        assert!(!needs_picker(Agent::ClaudeCode, 3, &two, now));
    }

    #[test]
    fn two_sessions_updated_within_10_minutes_need_picker() {
        let now = SystemTime::now();
        let cands = [candidate(now - Duration::from_secs(60)), candidate(now - Duration::from_secs(300))];
        assert!(needs_picker(Agent::Pi, 1, &cands, now));
        assert!(needs_picker(Agent::OpenCode, 1, &cands, now));
    }

    #[test]
    fn one_recent_and_one_old_session_need_no_picker() {
        let now = SystemTime::now();
        let cands = [candidate(now - Duration::from_secs(60)), candidate(now - Duration::from_secs(3600))];
        assert!(!needs_picker(Agent::Pi, 1, &cands, now));
    }

    #[test]
    fn replies_are_newest_first_and_at_most_25() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("s.jsonl");
        std::fs::write(&path, claude_transcript(30)).expect("transcript");
        let replies = load_replies(&SessionRef::Claude(path)).expect("replies");
        assert_eq!(replies.len(), REPLY_LIMIT);
        assert_eq!(replies[0].first_line(), "reply 30");
        assert_eq!(replies[24].first_line(), "reply 6");
        assert_eq!(replies[0].at.as_deref(), Some("2026-11-09T10:30:30.000Z"));
    }

    #[test]
    fn no_reply_yet_is_a_message_not_a_crash() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("s.jsonl");
        let only_prompt =
            r#"{"type":"user","uuid":"u1","parentUuid":null,"message":{"role":"user","content":"hello"}}"#;
        std::fs::write(&path, only_prompt).expect("transcript");
        let error = load_replies(&SessionRef::Claude(path)).expect_err("no reply");
        assert!(matches!(error, LoadError::NoReply));
        assert_eq!(error.to_string(), "the agent has no reply yet; press r to read the transcript again");
    }

    #[test]
    fn missing_session_names_what_was_searched() {
        let home = tempfile::tempdir().expect("home");
        let roots = Roots::from_lookup(home.path().to_path_buf(), |_| None);
        let source = Source { agent: Agent::Pi, cwd: PathBuf::from("/Users/dev/app"), pid: 1 };
        let error = candidates(&source, &roots, &[]).expect_err("nothing");
        let text = error.to_string();
        assert!(text.starts_with("no Pi session for /Users/dev/app in "), "{text}");
        assert!(text.ends_with(".pi/agent/sessions"), "{text}");
    }

    #[test]
    fn first_line_skips_blank_lines() {
        let reply = Reply { id: "r".into(), text: "\n  \n  ## Plan  \nbody".into(), at: None };
        assert_eq!(reply.first_line(), "## Plan");
    }

    /// Answers `rex` calls by the method name in argv.
    struct RexScript {
        calls: RefCell<Vec<String>>,
    }

    impl Runner for RexScript {
        fn run(&self, argv: &[OsString], _stdin: Option<&[u8]>) -> std::io::Result<Output> {
            let words: Vec<String> = argv.iter().map(|a| a.to_string_lossy().into_owned()).collect();
            self.calls.borrow_mut().push(words.join(" "));
            let session = words[4].as_str();
            let process = |argv0: &str, cwd: &str| {
                format!(r#"{{"foreground":{{"pid":1,"name":"node","argv0":"{argv0}","cwd":"{cwd}"}}}}"#)
            };
            let view = |blocks: &[&str]| {
                let blocks: Vec<String> = blocks
                    .iter()
                    .map(|b| {
                        format!(r#"{{"block_id":"{b}","label":"x","creator_name":"{TERMINAL_CREATOR}"}}"#)
                    })
                    .collect();
                format!(
                    r#"{{"windows":[{{"window_id":"w","layers":[{{"blocks":[{}]}}]}}]}}"#,
                    blocks.join(",")
                )
            };
            let stdout = match words.as_slice() {
                [_, _, _, _, _, method, _] if method == "session.list" => {
                    r#"{"sessions":[{"session_id":"session:a","label":"a"},{"session_id":"session:b","label":"b"}]}"#
                        .to_owned()
                }
                [_, _, _, _, _, method, _] if method == "session.view" => match session {
                    "session:a" => view(&["block:pi1", "block:shell"]),
                    _ => view(&["block:pi2", "block:pi-other-dir", "block:gone"]),
                },
                [_, _, _, _, _, _, block, method, _] if method == "process" => match block.as_str() {
                    "block:pi1" | "block:pi2" => process("pi", "/Users/dev/app"),
                    "block:pi-other-dir" => process("pi", "/Users/dev/other"),
                    "block:shell" => process("zsh", "/Users/dev/app"),
                    _ => {
                        return Ok(Output { status: ExitStatus::from_raw(1 << 8), stdout: vec![], stderr: vec![] });
                    }
                },
                _ => panic!("unexpected call: {words:?}"),
            };
            Ok(Output { status: ExitStatus::from_raw(0), stdout: stdout.into_bytes(), stderr: vec![] })
        }
    }

    #[test]
    fn two_panes_same_agent_same_dir_need_picker() {
        let rex =
            Rex::new(PathBuf::from("/rex"), "session:a".into(), RexScript { calls: RefCell::default() });
        let panes = matching_panes(&rex, Agent::Pi, Path::new("/Users/dev/app")).expect("panes");
        assert_eq!(panes, 2, "pi1 and pi2; not the shell, not the other directory, not the failed pane");
        let now = SystemTime::now();
        assert!(needs_picker(Agent::Pi, panes, &[candidate(now)], now));
        assert!(!needs_picker(Agent::Pi, 1, &[candidate(now)], now));
    }
}
