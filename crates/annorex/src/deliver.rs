//! The send sequence: detect the agent, check for a dialog, paste, and press Enter.

use std::time::Duration;

use crate::agent::detect;
use crate::payload::{bracketed, sanitize};
use crate::rex::{Modes, ProcessInfo, Rex, RexError, Runner, StatusList};
use crate::safety::{Safety, check};

/// The time between the paste and the Enter key. Some agents read a paste in parts.
pub const ENTER_DELAY: Duration = Duration::from_millis(300);

/// The Rex calls that delivery needs. Tests replace it with a fake.
pub trait RexApi {
    fn process(&self, block: &str) -> Result<ProcessInfo, RexError>;
    fn program_status(&self, block: &str) -> Result<StatusList, RexError>;
    fn modes(&self, block: &str) -> Result<Modes, RexError>;
    fn write(&self, block: &str, data: &str) -> Result<(), RexError>;
}

impl<R: Runner> RexApi for Rex<R> {
    fn process(&self, block: &str) -> Result<ProcessInfo, RexError> {
        Rex::process(self, block)
    }

    fn program_status(&self, block: &str) -> Result<StatusList, RexError> {
        Rex::program_status(self, block)
    }

    fn modes(&self, block: &str) -> Result<Modes, RexError> {
        Rex::modes(self, block)
    }

    fn write(&self, block: &str, data: &str) -> Result<(), RexError> {
        Rex::write(self, block, data)
    }
}

/// Waits. Tests replace it with a fake.
pub trait Sleeper {
    fn sleep(&self, d: Duration);
}

/// Waits with `std::thread::sleep`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealSleeper;

impl Sleeper for RealSleeper {
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// Paste and press Enter, or paste only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMode {
    Send,
    PasteOnly,
}

/// What delivery did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The paste and the Enter key were both written.
    Sent,
    /// The paste was written. The user presses Enter.
    Pasted,
}

/// Why delivery did not write, or failed.
#[derive(Debug, thiserror::Error)]
pub enum DeliverError {
    #[error("no supported agent (Claude Code, Pi, OpenCode) runs in the target pane")]
    NoAgent,
    #[error("the agent shows a dialog{}; answer it first", dialog_text(.kind.as_deref(), .msg.as_deref()))]
    Blocked { kind: Option<String>, msg: Option<String> },
    #[error(
        "the text is pasted, but Enter was not pressed: the agent opened a dialog{}; answer it, then press Enter",
        dialog_text(.kind.as_deref(), .msg.as_deref())
    )]
    EnterWithheld { kind: Option<String>, msg: Option<String> },
    #[error("the target pane has not turned on bracketed paste")]
    NoBracketedPaste,
    #[error("the payload is empty")]
    EmptyPayload,
    #[error(transparent)]
    Rex(#[from] RexError),
}

fn dialog_text(kind: Option<&str>, msg: Option<&str>) -> String {
    match (kind, msg) {
        (Some(kind), Some(msg)) => format!(" ({kind}: {msg})"),
        (Some(text), None) | (None, Some(text)) => format!(" ({text})"),
        (None, None) => String::new(),
    }
}

/// Deliver `payload` to the agent in `block`.
///
/// The order is: process, status, modes, status again, paste, wait `enter_delay`, status a
/// third time, Enter. Nothing is written when there is no agent, when the agent shows a dialog
/// at the first two checks, or when bracketed paste is off. When the paste fails, or a dialog
/// opens after the paste, Enter is not pressed. Delivery never tries again.
pub fn deliver(
    api: &impl RexApi,
    sleeper: &impl Sleeper,
    block: &str,
    payload: &str,
    mode: DeliveryMode,
    enter_delay: Duration,
) -> Result<Outcome, DeliverError> {
    if sanitize(payload).trim().is_empty() {
        return Err(DeliverError::EmptyPayload);
    }
    let process = api.process(block)?;
    let status = api.program_status(block)?;
    if detect(&process, &status).is_none() {
        return Err(DeliverError::NoAgent);
    }
    refuse_dialog(&status)?;
    if !api.modes(block)?.bracketed_paste() {
        return Err(DeliverError::NoBracketedPaste);
    }
    // A dialog can open while Annorex checks. Check again immediately before the paste.
    refuse_dialog(&api.program_status(block)?)?;
    api.write(block, &bracketed(payload))?;
    if mode == DeliveryMode::PasteOnly {
        return Ok(Outcome::Pasted);
    }
    sleeper.sleep(enter_delay);
    // A dialog can open during the wait. Enter would answer it, so check again.
    if let Safety::Blocked { kind, msg } = check(&api.program_status(block)?) {
        return Err(DeliverError::EnterWithheld { kind, msg });
    }
    api.write(block, "\r")?;
    Ok(Outcome::Sent)
}

fn refuse_dialog(status: &StatusList) -> Result<(), DeliverError> {
    match check(status) {
        Safety::Clear { .. } => Ok(()),
        Safety::Blocked { kind, msg } => Err(DeliverError::Blocked { kind, msg }),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::*;
    use crate::rex::{Mode, Modes, Proc, ProcessInfo, StatusList, StatusRecord};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Process,
        Status,
        Modes,
        Write(String),
        Sleep(Duration),
    }

    struct Fake {
        log: RefCell<Vec<Call>>,
        argv0: Option<&'static str>,
        states: RefCell<VecDeque<Option<&'static str>>>,
        paste_mode: bool,
        fail_first_write: bool,
    }

    impl Fake {
        fn new(states: &[Option<&'static str>]) -> Self {
            Self {
                log: RefCell::new(Vec::new()),
                argv0: Some("claude"),
                states: RefCell::new(states.iter().copied().collect()),
                paste_mode: true,
                fail_first_write: false,
            }
        }

        fn calls(&self) -> Vec<Call> {
            self.log.borrow().clone()
        }

        fn writes(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter_map(|call| if let Call::Write(data) = call { Some(data) } else { None })
                .collect()
        }
    }

    fn status_record(state: &str) -> StatusRecord {
        StatusRecord {
            id: None,
            state: state.to_owned(),
            kind: (state == "blocked").then(|| "question".to_owned()),
            app: Some("claude-code".to_owned()),
            msg: (state == "blocked").then(|| "answer: Pick a color.".to_owned()),
            updated_at: "2026-11-09T15:00:00+07:00".to_owned(),
            owner: None,
        }
    }

    impl RexApi for Fake {
        fn process(&self, _block: &str) -> Result<ProcessInfo, RexError> {
            self.log.borrow_mut().push(Call::Process);
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
            self.log.borrow_mut().push(Call::Status);
            let state = self.states.borrow_mut().pop_front().flatten();
            Ok(StatusList { records: state.map(status_record).into_iter().collect() })
        }

        fn modes(&self, _block: &str) -> Result<Modes, RexError> {
            self.log.borrow_mut().push(Call::Modes);
            let mode =
                Mode { name: "bracketed_paste".to_owned(), number: 2004, ansi: None, set: self.paste_mode };
            Ok(Modes { screen: "alternate".to_owned(), modes: Some(vec![mode]) })
        }

        fn write(&self, _block: &str, data: &str) -> Result<(), RexError> {
            let first = !self.log.borrow().iter().any(|call| matches!(call, Call::Write(_)));
            self.log.borrow_mut().push(Call::Write(data.to_owned()));
            if first && self.fail_first_write {
                return Err(RexError::Failed {
                    method: "write".to_owned(),
                    code: Some(1),
                    stderr: "gone".to_owned(),
                });
            }
            Ok(())
        }
    }

    impl Sleeper for Fake {
        fn sleep(&self, d: Duration) {
            self.log.borrow_mut().push(Call::Sleep(d));
        }
    }

    const BLOCK: &str = "block:b1";

    #[test]
    fn enter_delay_comes_from_the_caller() {
        let fake = Fake::new(&[]);
        let delay = Duration::from_millis(120);
        deliver(&fake, &fake, BLOCK, "note", DeliveryMode::Send, delay).expect("sent");
        assert!(fake.calls().contains(&Call::Sleep(delay)), "{:?}", fake.calls());
        assert!(!fake.calls().contains(&Call::Sleep(ENTER_DELAY)));
    }

    #[test]
    fn send_happy_path_order() {
        let fake = Fake::new(&[Some("idle"), Some("idle"), Some("idle")]);
        let outcome =
            deliver(&fake, &fake, BLOCK, "hello\r\nworld", DeliveryMode::Send, ENTER_DELAY).expect("sent");
        assert_eq!(outcome, Outcome::Sent);
        assert_eq!(
            fake.calls(),
            [
                Call::Process,
                Call::Status,
                Call::Modes,
                Call::Status,
                Call::Write("\x1b[200~hello\nworld\x1b[201~".to_owned()),
                Call::Sleep(Duration::from_millis(300)),
                Call::Status,
                Call::Write("\r".to_owned()),
            ]
        );
    }

    #[test]
    fn sends_while_working() {
        let fake = Fake::new(&[Some("working"), Some("working"), Some("working")]);
        assert_eq!(
            deliver(&fake, &fake, BLOCK, "note", DeliveryMode::Send, ENTER_DELAY).expect("sent"),
            Outcome::Sent
        );
        assert_eq!(fake.writes().len(), 2);
    }

    #[test]
    fn paste_only_never_presses_enter() {
        let fake = Fake::new(&[Some("idle"), Some("idle")]);
        assert_eq!(
            deliver(&fake, &fake, BLOCK, "note", DeliveryMode::PasteOnly, ENTER_DELAY).expect("pasted"),
            Outcome::Pasted
        );
        assert_eq!(fake.writes(), ["\x1b[200~note\x1b[201~"]);
        assert!(!fake.calls().iter().any(|call| matches!(call, Call::Sleep(_))));
    }

    #[test]
    fn refuses_when_blocked() {
        let fake = Fake::new(&[Some("blocked"), Some("blocked")]);
        match deliver(&fake, &fake, BLOCK, "note", DeliveryMode::Send, ENTER_DELAY) {
            Err(DeliverError::Blocked { kind, msg }) => {
                assert_eq!(kind.as_deref(), Some("question"));
                assert_eq!(msg.as_deref(), Some("answer: Pick a color."));
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
        assert_eq!(fake.writes(), [] as [String; 0]);
    }

    #[test]
    fn refuses_when_dialog_opens_between_checks() {
        let fake = Fake::new(&[Some("working"), Some("blocked")]);
        assert!(matches!(
            deliver(&fake, &fake, BLOCK, "note", DeliveryMode::Send, ENTER_DELAY),
            Err(DeliverError::Blocked { .. })
        ));
        assert_eq!(fake.writes(), [] as [String; 0]);
    }

    #[test]
    fn withholds_enter_when_dialog_opens_after_the_paste() {
        let fake = Fake::new(&[Some("working"), Some("working"), Some("blocked")]);
        match deliver(&fake, &fake, BLOCK, "note", DeliveryMode::Send, ENTER_DELAY) {
            Err(DeliverError::EnterWithheld { kind, .. }) => assert_eq!(kind.as_deref(), Some("question")),
            other => panic!("expected EnterWithheld, got {other:?}"),
        }
        assert_eq!(fake.writes(), ["\x1b[200~note\x1b[201~"]);
        assert_eq!(fake.calls().last(), Some(&Call::Status));
    }

    #[test]
    fn refuses_without_bracketed_paste() {
        let mut fake = Fake::new(&[Some("idle"), Some("idle")]);
        fake.paste_mode = false;
        assert!(matches!(
            deliver(&fake, &fake, BLOCK, "note", DeliveryMode::Send, ENTER_DELAY),
            Err(DeliverError::NoBracketedPaste)
        ));
        assert_eq!(fake.writes(), [] as [String; 0]);
    }

    #[test]
    fn no_agent_refuses() {
        let mut fake = Fake::new(&[None, None]);
        fake.argv0 = Some("-zsh");
        assert!(matches!(
            deliver(&fake, &fake, BLOCK, "note", DeliveryMode::Send, ENTER_DELAY),
            Err(DeliverError::NoAgent)
        ));
        assert_eq!(fake.writes(), [] as [String; 0]);
    }

    #[test]
    fn empty_payload_refuses() {
        let fake = Fake::new(&[Some("idle"), Some("idle")]);
        for payload in ["", "\x1b\x07", " \n\t\r\n"] {
            assert!(matches!(
                deliver(&fake, &fake, BLOCK, payload, DeliveryMode::Send, ENTER_DELAY),
                Err(DeliverError::EmptyPayload)
            ));
        }
        assert!(fake.calls().is_empty(), "an empty payload needs no Rex call");
    }

    #[test]
    fn failed_paste_does_not_press_enter() {
        let mut fake = Fake::new(&[Some("idle"), Some("idle")]);
        fake.fail_first_write = true;
        assert!(matches!(
            deliver(&fake, &fake, BLOCK, "note", DeliveryMode::Send, ENTER_DELAY),
            Err(DeliverError::Rex(_))
        ));
        assert_eq!(fake.writes().len(), 1);
    }
}
