//! Which coding agent runs in a pane.

use annorex_hosts::Host;

use crate::rex::{ProcessInfo, StatusList};

/// A supported coding agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    ClaudeCode,
    Pi,
    OpenCode,
}

/// What a message does when it arrives while the agent works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusyEffect {
    /// The agent keeps the message and starts it as the next turn.
    Queued,
    /// The agent puts the message into the current task after its tool calls.
    Steers,
}

impl Agent {
    const ALL: [Agent; 3] = [Agent::ClaudeCode, Agent::Pi, Agent::OpenCode];

    /// The `app` value of the agent's root status record.
    pub fn status_app(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Pi => "pi",
            Self::OpenCode => "opencode",
        }
    }

    /// The `argv0` of the agent's foreground process.
    pub fn argv0(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Pi => "pi",
            Self::OpenCode => "opencode",
        }
    }

    /// The transcript reader of the agent.
    pub fn host(self) -> Host {
        match self {
            Self::ClaudeCode => Host::ClaudeCode,
            Self::Pi => Host::Pi,
            Self::OpenCode => Host::OpenCode,
        }
    }

    /// What a message does when it arrives while the agent works.
    pub fn busy_effect(self) -> BusyEffect {
        match self {
            Self::ClaudeCode => BusyEffect::Queued,
            Self::Pi | Self::OpenCode => BusyEffect::Steers,
        }
    }
}

/// Find the agent in a pane.
///
/// The `app` of the root status record names the agent only when the foreground process
/// confirms it: the record owner is the foreground process, or the `argv0` of the foreground
/// process names the same agent. Rex keeps `done` and `idle` records after the agent exits, so a
/// record alone can name an agent that no longer runs. Without a confirmed record, the `argv0`
/// of the foreground process decides. The process name is never used: Pi runs as `node`.
pub fn detect(process: &ProcessInfo, status: &StatusList) -> Option<Agent> {
    let foreground = process.foreground.as_ref()?;
    let by_argv0 = foreground
        .argv0
        .as_deref()
        .and_then(|argv0| Agent::ALL.into_iter().find(|agent| agent.argv0() == argv0));
    let by_app = status.root().and_then(|root| {
        let agent = Agent::ALL.into_iter().find(|agent| Some(agent.status_app()) == root.app.as_deref())?;
        let owner_runs = root.owner.as_ref().is_some_and(|owner| owner.pid == foreground.pid);
        (owner_runs || by_argv0 == Some(agent)).then_some(agent)
    });
    by_app.or(by_argv0)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use crate::rex::{ProcessInfo, StatusList};

    fn process(name: &str) -> ProcessInfo {
        let text = match name {
            "claude" => include_str!("../tests/fixtures/rex/process-claude.json"),
            "pi" => include_str!("../tests/fixtures/rex/process-pi.json"),
            "opencode" => include_str!("../tests/fixtures/rex/process-opencode.json"),
            _ => include_str!("../tests/fixtures/rex/process-shell.json"),
        };
        serde_json::from_str(text).expect("process fixture")
    }

    fn status(name: &str) -> StatusList {
        let text = match name {
            "claude" => include_str!("../tests/fixtures/rex/status-claude-working.json"),
            "pi" => include_str!("../tests/fixtures/rex/status-pi-idle.json"),
            "opencode" => include_str!("../tests/fixtures/rex/status-opencode-idle.json"),
            _ => include_str!("../tests/fixtures/rex/status-empty.json"),
        };
        serde_json::from_str(text).expect("status fixture")
    }

    fn empty() -> StatusList {
        StatusList { records: Vec::new() }
    }

    #[test]
    fn detects_each_agent_from_status_app() {
        assert_eq!(detect(&process("claude"), &status("claude")), Some(Agent::ClaudeCode));
        assert_eq!(detect(&process("pi"), &status("pi")), Some(Agent::Pi));
        assert_eq!(detect(&process("opencode"), &status("opencode")), Some(Agent::OpenCode));
    }

    #[test]
    fn status_app_with_the_owner_as_foreground_decides() {
        // argv0 does not name an agent, but the record owner is the foreground process.
        let mut pi = process("pi");
        if let Some(fg) = pi.foreground.as_mut() {
            fg.argv0 = Some("node".to_owned());
        }
        assert_eq!(detect(&pi, &status("pi")), Some(Agent::Pi));
    }

    #[test]
    fn leftover_record_with_a_shell_in_the_foreground_is_not_an_agent() {
        // Rex keeps `done` and `idle` records after the agent exits to the shell.
        let shell = process("shell");
        for name in ["claude", "pi", "opencode"] {
            let mut leftover = status(name);
            for record in &mut leftover.records {
                record.state = "done".to_owned();
            }
            assert_eq!(detect(&shell, &leftover), None, "{name}");
        }
    }

    #[test]
    fn falls_back_to_argv0_without_status() {
        assert_eq!(detect(&process("claude"), &empty()), Some(Agent::ClaudeCode));
        assert_eq!(detect(&process("pi"), &empty()), Some(Agent::Pi));
        assert_eq!(detect(&process("opencode"), &empty()), Some(Agent::OpenCode));
    }

    #[test]
    fn plain_shell_is_not_an_agent() {
        assert_eq!(detect(&process("shell"), &status("shell")), None);
    }

    #[test]
    fn busy_effect_per_agent() {
        assert_eq!(Agent::ClaudeCode.busy_effect(), BusyEffect::Queued);
        assert_eq!(Agent::Pi.busy_effect(), BusyEffect::Steers);
        assert_eq!(Agent::OpenCode.busy_effect(), BusyEffect::Steers);
    }

    #[test]
    fn status_app_and_host_per_agent() {
        assert_eq!(Agent::ClaudeCode.status_app(), "claude-code");
        assert_eq!(Agent::Pi.status_app(), "pi");
        assert_eq!(Agent::OpenCode.status_app(), "opencode");
        assert_eq!(Agent::ClaudeCode.host(), Host::ClaudeCode);
        assert_eq!(Agent::Pi.host(), Host::Pi);
        assert_eq!(Agent::OpenCode.host(), Host::OpenCode);
    }

    #[test]
    fn foreground_name_is_never_used() {
        let mut shell = process("shell");
        if let Some(fg) = shell.foreground.as_mut() {
            fg.name = "opencode".to_owned();
        }
        assert_eq!(detect(&shell, &empty()), None);
    }
}
