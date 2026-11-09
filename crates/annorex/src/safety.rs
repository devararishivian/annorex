//! The dialog check: Annorex refuses to send while the agent shows a dialog, because the
//! paste and the Enter key would become the answer of the dialog.

use crate::rex::StatusList;

/// The result of the dialog check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Safety {
    /// No dialog. `state` is the root state, or `None` when there is no root record.
    Clear { state: Option<String> },
    /// The agent shows a dialog.
    Blocked { kind: Option<String>, msg: Option<String> },
}

impl Safety {
    pub fn allows_send(&self) -> bool {
        matches!(self, Self::Clear { .. })
    }
}

/// Only a root record with the state `blocked` blocks. Other states, unknown states, and a
/// missing record are clear: the `program_status` schema says to treat an unknown value as idle.
pub fn check(status: &StatusList) -> Safety {
    match status.root() {
        Some(root) if root.state == "blocked" => {
            Safety::Blocked { kind: root.kind.clone(), msg: root.msg.clone() }
        }
        Some(root) => Safety::Clear { state: Some(root.state.clone()) },
        None => Safety::Clear { state: None },
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use crate::rex::{StatusList, StatusRecord};

    fn record(id: Option<&str>, state: &str) -> StatusRecord {
        StatusRecord {
            id: id.map(str::to_owned),
            state: state.to_owned(),
            kind: None,
            app: Some("claude-code".to_owned()),
            msg: None,
            updated_at: "2026-11-09T15:00:00+07:00".to_owned(),
            owner: None,
        }
    }

    fn list(records: Vec<StatusRecord>) -> StatusList {
        StatusList { records }
    }

    #[test]
    fn blocked_record_is_blocked() {
        let status: StatusList =
            serde_json::from_str(include_str!("../tests/fixtures/rex/status-claude-blocked.json"))
                .expect("fixture");
        let safety = check(&status);
        assert_eq!(
            safety,
            Safety::Blocked {
                kind: Some("question".to_owned()),
                msg: Some("answer: Pick a color. (Red · Blue)".to_owned()),
            }
        );
        assert!(!safety.allows_send());
    }

    #[test]
    fn working_idle_and_done_are_clear() {
        for state in ["working", "idle", "done"] {
            let safety = check(&list(vec![record(None, state)]));
            assert_eq!(safety, Safety::Clear { state: Some(state.to_owned()) });
            assert!(safety.allows_send());
        }
    }

    #[test]
    fn unknown_state_is_clear() {
        let safety = check(&list(vec![record(None, "paused")]));
        assert!(safety.allows_send());
    }

    #[test]
    fn missing_record_is_clear() {
        let status: StatusList =
            serde_json::from_str(include_str!("../tests/fixtures/rex/status-empty.json")).expect("fixture");
        assert_eq!(check(&status), Safety::Clear { state: None });
    }

    #[test]
    fn blocked_child_record_is_ignored() {
        let safety = check(&list(vec![record(Some("build"), "blocked"), record(None, "idle")]));
        assert_eq!(safety, Safety::Clear { state: Some("idle".to_owned()) });
    }
}
