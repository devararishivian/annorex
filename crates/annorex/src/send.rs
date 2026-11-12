//! Send the notes to the agent and show what happened.

use std::time::Duration;

use crate::deliver::{DeliverError, DeliveryMode, Outcome, RexApi, Sleeper, deliver};
use crate::ui::App;

/// The limit of the delivery (spec §7.4). Annorex cannot see the input box of the agent.
const PROMPT_LIMIT: &str = "If the agent prompt had text, the notes joined it.";

/// What `send_review` did. The strings are the messages of the footer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendResult {
    /// The paste and the Enter key were written.
    Sent,
    /// The paste was written. The user presses Enter.
    Pasted,
    /// The paste was written, but Enter was held back because a dialog opened. The text is in the
    /// agent. The message tells the user what to do.
    Held(String),
    /// There was nothing to send.
    Nothing,
    /// A dialog is open, so nothing was written.
    Refused(String),
    /// Nothing was written, or the write failed.
    Failed(String),
}

fn dialog(kind: Option<&str>) -> String {
    kind.map_or_else(|| "a dialog".to_owned(), |kind| format!("a {kind} dialog"))
}

/// Deliver the export to the agent in `target`.
pub fn send_review(
    api: &impl RexApi,
    sleeper: &impl Sleeper,
    target: &str,
    export: &str,
    mode: DeliveryMode,
    enter_delay: Duration,
) -> SendResult {
    match deliver(api, sleeper, target, export, mode, enter_delay) {
        Ok(Outcome::Sent) => SendResult::Sent,
        Ok(Outcome::Pasted) => SendResult::Pasted,
        Err(DeliverError::EmptyPayload) => SendResult::Nothing,
        Err(DeliverError::Blocked { kind, .. }) => SendResult::Refused(format!(
            "Not sent: the agent shows {}. Your notes are kept.",
            dialog(kind.as_deref())
        )),
        Err(DeliverError::EnterWithheld { kind, .. }) => SendResult::Held(format!(
            "Pasted, but Enter was held: the agent opened {}. Answer it, then press Enter in the agent pane.",
            dialog(kind.as_deref())
        )),
        Err(error) => SendResult::Failed(format!("Not sent: {error}. Your notes are kept.")),
    }
}

/// Show the result in the app. On `Sent`, `Pasted`, and `Held` the notes count as sent at `now`.
/// Returns true when the draft must be saved.
pub fn apply_result(app: &mut App, result: SendResult, now: &str) -> bool {
    match result {
        SendResult::Sent | SendResult::Pasted => {
            let word = if result == SendResult::Sent { "Sent" } else { "Pasted" };
            app.finish_send(now);
            app.set_status(format!("{word}. {PROMPT_LIMIT}"));
            app.quit_if_pending();
            true
        }
        SendResult::Held(message) => {
            // The notes are in the input box of the agent. Stay open so the message shows.
            app.finish_send(now);
            app.set_status(message);
            app.cancel_pending_quit();
            true
        }
        SendResult::Nothing => {
            app.set_status("Nothing to send: no annotations yet.".to_owned());
            app.cancel_pending_quit();
            false
        }
        SendResult::Refused(message) | SendResult::Failed(message) => {
            app.set_status(message);
            app.cancel_pending_quit();
            false
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::*;
    use crate::rex::{Mode, Modes, Owner, Proc, ProcessInfo, RexError, StatusList, StatusRecord};

    /// A Rex that answers like a pane with Claude Code. `states` are the answers of the status
    /// checks in order; the last one repeats.
    struct Fake {
        argv0: Option<&'static str>,
        states: RefCell<VecDeque<&'static str>>,
        writes: RefCell<Vec<String>>,
        sleeps: RefCell<Vec<Duration>>,
    }

    impl Fake {
        fn new(states: &[&'static str]) -> Self {
            Self {
                argv0: Some("claude"),
                states: RefCell::new(states.iter().copied().collect()),
                writes: RefCell::new(Vec::new()),
                sleeps: RefCell::new(Vec::new()),
            }
        }
    }

    impl RexApi for Fake {
        fn process(&self, _block: &str) -> Result<ProcessInfo, RexError> {
            let foreground = Proc {
                pid: 10,
                name: "node".to_owned(),
                argv0: self.argv0.map(str::to_owned),
                cwd: None,
                path: None,
            };
            Ok(ProcessInfo { foreground: Some(foreground), child: None })
        }

        fn program_status(&self, _block: &str) -> Result<StatusList, RexError> {
            let mut states = self.states.borrow_mut();
            let state = if states.len() > 1 { states.pop_front() } else { states.front().copied() };
            let record = state.map(|state| StatusRecord {
                id: None,
                state: state.to_owned(),
                kind: (state == "blocked").then(|| "permission".to_owned()),
                app: Some("claude-code".to_owned()),
                msg: None,
                updated_at: "2026-11-09T15:00:00+07:00".to_owned(),
                owner: Some(Owner { pid: 10, name: "claude".to_owned() }),
            });
            Ok(StatusList { records: record.into_iter().collect() })
        }

        fn modes(&self, _block: &str) -> Result<Modes, RexError> {
            let mode = Mode { name: "bracketed_paste".to_owned(), number: 2004, ansi: None, set: true };
            Ok(Modes { screen: "alternate".to_owned(), modes: Some(vec![mode]) })
        }

        fn write(&self, _block: &str, data: &str) -> Result<(), RexError> {
            self.writes.borrow_mut().push(data.to_owned());
            Ok(())
        }
    }

    impl Sleeper for Fake {
        fn sleep(&self, d: Duration) {
            self.sleeps.borrow_mut().push(d);
        }
    }

    const EXPORT: &str = "# Annotations on last reply\n\n## Annotation 1 (line 1)\nRemove this: \"x\"\n";

    fn run(fake: &Fake, mode: DeliveryMode) -> SendResult {
        send_review(fake, fake, "block:b1", EXPORT, mode, Duration::from_millis(120))
    }

    #[test]
    fn send_review_gives_the_export_to_deliver() {
        let fake = Fake::new(&["idle"]);
        assert_eq!(run(&fake, DeliveryMode::Send), SendResult::Sent);
        let writes = fake.writes.borrow();
        assert_eq!(writes.len(), 2);
        assert!(writes[0].starts_with("\x1b[200~") && writes[0].contains("Remove this: \"x\""), "{writes:?}");
        assert!(writes[0].ends_with("\x1b[201~"));
        assert_eq!(writes[1], "\r");
        assert_eq!(*fake.sleeps.borrow(), [Duration::from_millis(120)]);
    }

    #[test]
    fn paste_only_does_not_press_enter() {
        let fake = Fake::new(&["idle"]);
        assert_eq!(run(&fake, DeliveryMode::PasteOnly), SendResult::Pasted);
        assert_eq!(fake.writes.borrow().len(), 1);
    }

    #[test]
    fn a_dialog_refuses_the_send() {
        let fake = Fake::new(&["idle", "blocked"]);
        let result = run(&fake, DeliveryMode::Send);
        assert_eq!(
            result,
            SendResult::Refused(
                "Not sent: the agent shows a permission dialog. Your notes are kept.".to_owned()
            )
        );
        assert!(fake.writes.borrow().is_empty(), "nothing was written");
    }

    #[test]
    fn enter_held_is_reported() {
        let fake = Fake::new(&["idle", "idle", "blocked"]);
        let result = run(&fake, DeliveryMode::Send);
        assert_eq!(
            result,
            SendResult::Held(
                "Pasted, but Enter was held: the agent opened a permission dialog. Answer it, then press Enter in the agent pane."
                    .to_owned()
            )
        );
        assert_eq!(fake.writes.borrow().len(), 1, "only the paste");
    }

    #[test]
    fn empty_review_sends_nothing() {
        let fake = Fake::new(&["idle"]);
        let result = send_review(&fake, &fake, "block:b1", "", DeliveryMode::Send, Duration::from_millis(1));
        assert_eq!(result, SendResult::Nothing);
        assert!(fake.writes.borrow().is_empty());
    }

    #[test]
    fn other_errors_keep_the_notes() {
        // No agent: no record and no agent name in argv0.
        let mut fake = Fake::new(&[]);
        fake.argv0 = None;
        match run(&fake, DeliveryMode::Send) {
            SendResult::Failed(message) => {
                assert!(
                    message.starts_with("Not sent: ") && message.ends_with(". Your notes are kept."),
                    "{message}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn refused_send_through_deliver_keeps_the_review_and_the_draft() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        use crate::agent::Agent;
        use crate::config::Config;
        use crate::replies::Reply;
        use crate::safety::Safety;
        use crate::ui::{AgentState, App, Screen};

        let state = AgentState { agent: Some(Agent::ClaudeCode), safety: Safety::Clear { state: None } };
        let reply = Reply { id: "r1".to_owned(), text: "alpha beta\n".to_owned(), at: None };
        let mut app = App::new(state, Screen::Viewer, vec![reply], &Config::default());
        // The view starts without a focus: `j` focuses the first block.
        for c in "jcnote\n".chars() {
            let code = if c == '\n' { KeyCode::Enter } else { KeyCode::Char(c) };
            app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        }
        let before = app.draft(Agent::ClaudeCode, "s").expect("draft");
        let fake = Fake::new(&["idle", "blocked"]);
        let result = send_review(&fake, &fake, "block:b1", &app.export(), DeliveryMode::Send, Duration::ZERO);
        assert!(matches!(result, SendResult::Refused(_)), "{result:?}");
        assert!(!apply_result(&mut app, result, "2026-11-09T12:00:00Z"), "no draft save after a refusal");
        let after = app.draft(Agent::ClaudeCode, "s").expect("draft");
        assert_eq!(after.review, before.review, "the annotations did not change");
        assert!(after.sent_at.is_none() && after.review.annotations.iter().all(|a| a.sent_at.is_none()));
        assert!(fake.writes.borrow().is_empty());
    }
}
