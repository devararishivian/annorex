//! The terminal loop of the viewer and the `annorex last` command.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, SystemTime};

use annorex_hosts::roots::Roots;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};

use crate::agent::{Agent, NOT_AGENT, detect};
use crate::open::pane_label;
use crate::replies::{
    Candidate, Reply, SessionRef, Source, candidates, load_replies, matching_panes, needs_picker,
    process_table,
};
use crate::rex::{Rex, RexError, Runner, SystemRunner};
use crate::safety::{Safety, check};
use crate::ui::{AgentState, App, Effect, Screen, draw};

/// The longest wait for an event before the loop draws again.
pub const WAIT: Duration = Duration::from_millis(250);
/// The time between two reads of the agent state.
pub const STATUS_EVERY: Duration = Duration::from_secs(2);

/// A source of terminal events.
pub trait EventSource {
    /// The next event, or `None` when no event comes in `timeout`.
    fn next(&mut self, timeout: Duration) -> io::Result<Option<Event>>;
}

/// The events of the controlling terminal.
#[derive(Debug, Default)]
pub struct CrosstermEvents;

impl EventSource for CrosstermEvents {
    fn next(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        if event::poll(timeout)? { event::read().map(Some) } else { Ok(None) }
    }
}

/// A place to draw the viewer.
pub trait Canvas {
    fn draw(&mut self, app: &mut App) -> io::Result<()>;
}

impl Canvas for DefaultTerminal {
    fn draw(&mut self, app: &mut App) -> io::Result<()> {
        ratatui::Terminal::draw(self, |frame| draw(frame, app)).map(|_| ())
    }
}

/// Run the viewer until `q`, a signal (`stop`), or the end of the terminal input. Each turn
/// waits for one event, applies all queued events and state updates, and then draws one time.
/// An error from `events` stops the loop with `Ok(())`. The caller restores the terminal.
pub fn run_loop(
    app: &mut App,
    events: &mut impl EventSource,
    canvas: &mut impl Canvas,
    stop: &AtomicBool,
    updates: &Receiver<AgentState>,
    effects: &mut impl FnMut(Effect, &mut App),
) -> io::Result<()> {
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        let Ok(mut pending) = events.next(WAIT) else { return Ok(()) };
        while let Some(event) = pending {
            if let Event::Key(key) = event {
                match app.handle_key(key) {
                    Effect::None => {}
                    Effect::Quit => return Ok(()),
                    effect => effects(effect, app),
                }
            }
            let Ok(next) = events.next(Duration::ZERO) else { return Ok(()) };
            pending = next;
        }
        while let Ok(state) = updates.try_recv() {
            app.set_state(state);
        }
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        canvas.draw(app)?;
    }
}

/// Start the terminal. When `init` fails, call `restore`: a half-done init can leave raw mode on.
fn start_terminal<T>(init: impl FnOnce() -> io::Result<T>, restore: impl FnOnce()) -> io::Result<T> {
    init().inspect_err(|_| restore())
}

/// `annorex last`: show the replies of the agent in `target` in this terminal. Every problem after
/// the Rex discovery shows in the pane, so the user can read it.
pub fn run_last(rex: &Rex<SystemRunner>, target: &str) -> io::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [SIGHUP, SIGTERM, SIGINT] {
        signal_hook::flag::register(signal, Arc::clone(&stop))?;
    }
    let mut viewer = Viewer {
        rex,
        target,
        state: AgentState { agent: None, safety: Safety::Clear { state: None } },
        session: None,
        candidates: Vec::new(),
    };
    let (screen, replies) = viewer.start_or_error();
    let mut app = App::new(viewer.state.clone(), screen, replies);

    let (sender, updates) = mpsc::channel();
    let watcher = Rex::new(rex.bin.clone(), rex.session.clone(), SystemRunner);
    let pane = target.to_owned();
    std::thread::spawn(move || watch_status(&watcher, &pane, &sender));

    let mut terminal = start_terminal(ratatui::try_init, ratatui::restore)?;
    let result = run_loop(
        &mut app,
        &mut CrosstermEvents,
        &mut terminal,
        &stop,
        &updates,
        &mut |effect, app: &mut App| viewer.apply(effect, app),
    );
    ratatui::restore();
    result
}

/// The agent and its state in a pane. A failed Rex call gives `None`.
fn read_state<R: Runner>(rex: &Rex<R>, target: &str) -> Result<AgentState, RexError> {
    let process = rex.process(target)?;
    let status = rex.program_status(target)?;
    Ok(AgentState { agent: detect(&process, &status), safety: check(&status) })
}

/// Send the state of the agent every `STATUS_EVERY`. Stop when the receiver is gone.
fn watch_status<R: Runner>(rex: &Rex<R>, target: &str, sender: &Sender<AgentState>) {
    loop {
        std::thread::sleep(STATUS_EVERY);
        let Ok(state) = read_state(rex, target) else { continue };
        if sender.send(state).is_err() {
            return;
        }
    }
}

/// The session of the viewer and the effects of its keys.
struct Viewer<'a, R: Runner> {
    rex: &'a Rex<R>,
    target: &'a str,
    /// The agent and its state at the last start. `agent` is `None` before the first start.
    state: AgentState,
    /// The session that the viewer shows. `None` before the user picks one.
    session: Option<SessionRef>,
    candidates: Vec<Candidate>,
}

impl<R: Runner> Viewer<'_, R> {
    /// `start`, with a failed Rex call as a message for the pane.
    fn start_or_error(&mut self) -> (Screen, Vec<Reply>) {
        self.start().unwrap_or_else(|error| (Screen::Error(error.to_string()), Vec::new()))
    }

    /// Find the agent and its session, and read its replies. Only a failed Rex call is an error.
    /// Other problems give `Screen::Error`.
    fn start(&mut self) -> Result<(Screen, Vec<Reply>), RexError> {
        self.session = None;
        self.state = read_state(self.rex, self.target)?;
        let Some(agent) = self.state.agent else {
            return Ok((Screen::Error(NOT_AGENT.to_owned()), Vec::new()));
        };
        let process = self.rex.process(self.target)?;
        let Some(source) = Source::from_process(agent, &process) else {
            let message = "Rex does not report the directory of the agent process".to_owned();
            return Ok((Screen::Error(message), Vec::new()));
        };
        let table = match process_table() {
            Ok(table) => table,
            Err(error) => {
                return Ok((Screen::Error(format!("cannot read the process table: {error}")), Vec::new()));
            }
        };
        let found = match candidates(&source, &Roots::from_env(), &table) {
            Ok(found) => found,
            Err(error) => return Ok((Screen::Error(error.to_string()), Vec::new())),
        };
        let panes =
            if agent == Agent::ClaudeCode { 1 } else { matching_panes(self.rex, agent, &source.cwd)? };
        self.candidates = found;
        if needs_picker(agent, panes, &self.candidates, SystemTime::now()) {
            return Ok((Screen::Picker(self.candidates.clone()), Vec::new()));
        }
        Ok(self.open(0))
    }

    /// Read the replies of candidate `index`.
    fn open(&mut self, index: usize) -> (Screen, Vec<Reply>) {
        let Some(candidate) = self.candidates.get(index) else {
            return (Screen::Error("the session is gone; press r to look again".to_owned()), Vec::new());
        };
        self.session = Some(candidate.session.clone());
        match load_replies(&candidate.session) {
            Ok(replies) => (Screen::Viewer, replies),
            Err(error) => (Screen::Error(error.to_string()), Vec::new()),
        }
    }

    fn apply(&mut self, effect: Effect, app: &mut App) {
        match effect {
            Effect::None | Effect::Quit => {}
            Effect::Reload => self.reload(app),
            Effect::PickSession(index) => {
                let (screen, replies) = self.open(index);
                show(app, screen, replies);
            }
            Effect::ToggleZoom => {
                if let Err(message) = self.toggle_zoom() {
                    app.set_status(message);
                }
            }
        }
    }

    /// Read the replies again. Claude Code finds its session again by pid, because the session
    /// file can change while the agent runs. Pi and `OpenCode` keep the session that the user saw.
    fn reload(&mut self, app: &mut App) {
        let showing = matches!(app.screen, Screen::Viewer);
        let result = match &self.session {
            Some(session) if self.state.agent != Some(Agent::ClaudeCode) => Ok(match load_replies(session) {
                Ok(replies) => (Screen::Viewer, replies),
                Err(error) => (Screen::Error(error.to_string()), Vec::new()),
            }),
            _ => self.start(),
        };
        match result {
            // Keep the reply on the screen. The footer tells why the reload failed.
            Ok((Screen::Error(message), _)) if showing => app.set_status(message),
            Ok((screen, replies)) => {
                app.cache.clear();
                show(app, screen, replies);
            }
            Err(error) if showing => app.set_status(error.to_string()),
            Err(error) => app.set_error(error.to_string()),
        }
        app.set_state(self.state.clone());
    }

    fn toggle_zoom(&self) -> Result<(), String> {
        let label = pane_label(self.target);
        let view = self.rex.view().map_err(|error| format!("cannot zoom: {error}"))?;
        let block = view
            .find_label(&label)
            .ok_or_else(|| "cannot zoom: Rex does not show the viewer pane".to_owned())?;
        self.rex.zoom_toggle(&block.block_id).map_err(|error| format!("cannot zoom: {error}"))
    }
}

/// Put a new screen in the app. Replies go through `set_replies`, which keeps the selected reply.
fn show(app: &mut App, screen: Screen, replies: Vec<Reply>) {
    match screen {
        Screen::Viewer => app.set_replies(replies),
        Screen::Error(message) => app.set_error(message),
        picker @ Screen::Picker(_) => app.screen = picker,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::collections::VecDeque;
    use std::io;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::{Canvas, EventSource, run_loop};
    use crate::agent::Agent;
    use crate::replies::Reply;
    use crate::safety::Safety;
    use crate::ui::{AgentState, App, Effect, Screen, draw};

    /// Gives the scripted results in order. `Ok(None)` is a timeout. At the end, it gives `EIO`.
    struct Script(VecDeque<io::Result<Option<Event>>>);

    impl EventSource for Script {
        fn next(&mut self, _timeout: Duration) -> io::Result<Option<Event>> {
            self.0.pop_front().unwrap_or_else(|| Err(io::Error::from_raw_os_error(5)))
        }
    }

    /// Draws into a test backend and counts the draws.
    struct Counting {
        terminal: Terminal<TestBackend>,
        draws: usize,
    }

    impl Counting {
        fn new() -> Self {
            Self { terminal: Terminal::new(TestBackend::new(80, 24)).expect("terminal"), draws: 0 }
        }

        fn header(&self) -> String {
            let buffer = self.terminal.backend().buffer();
            (0..80).map(|x| buffer.cell((x, 0)).map_or(" ", |cell| cell.symbol())).collect()
        }
    }

    impl Canvas for Counting {
        fn draw(&mut self, app: &mut App) -> io::Result<()> {
            self.draws += 1;
            self.terminal.draw(|frame| draw(frame, app)).map(|_| ()).map_err(|never| match never {})
        }
    }

    fn app() -> App {
        let state = AgentState {
            agent: Some(Agent::ClaudeCode),
            safety: Safety::Clear { state: Some("idle".into()) },
        };
        let reply = Reply { id: "r1".into(), text: "# Title\n\nSome text.".into(), at: None };
        App::new(state, Screen::Viewer, vec![reply])
    }

    fn script(items: Vec<io::Result<Option<Event>>>) -> Script {
        Script(items.into())
    }

    #[allow(clippy::unnecessary_wraps, reason = "a script item has the type of an EventSource result")]
    fn key(c: char) -> io::Result<Option<Event>> {
        Ok(Some(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))))
    }

    fn no_effects() -> impl FnMut(Effect, &mut App) {
        |effect, _| panic!("unexpected effect {effect:?}")
    }

    #[test]
    fn failed_init_restores_the_terminal() {
        let restored = std::cell::Cell::new(false);
        let result: io::Result<u8> =
            super::start_terminal(|| Err(io::Error::other("no alternate screen")), || restored.set(true));
        assert!(result.is_err());
        assert!(restored.get(), "restore must run when the init fails");
        let restored = std::cell::Cell::new(false);
        let result = super::start_terminal(|| Ok(7_u8), || restored.set(true));
        assert_eq!(result.expect("init"), 7);
        assert!(!restored.get(), "restore must not run when the init works");
    }

    #[test]
    fn many_resizes_cost_one_draw() {
        let resize = || Ok(Some(Event::Resize(100, 30)));
        let mut events = script(vec![resize(), resize(), resize(), resize(), resize(), Ok(None)]);
        let mut canvas = Counting::new();
        let (_tx, rx) = mpsc::channel();
        let stop = AtomicBool::new(false);
        run_loop(&mut app(), &mut events, &mut canvas, &stop, &rx, &mut no_effects()).expect("loop");
        assert_eq!(canvas.draws, 1);
    }

    #[test]
    fn stop_flag_ends_loop() {
        let mut events = script(vec![Ok(None), Ok(None), Ok(None)]);
        let mut canvas = Counting::new();
        let (_tx, rx) = mpsc::channel();
        let stop = AtomicBool::new(true);
        run_loop(&mut app(), &mut events, &mut canvas, &stop, &rx, &mut no_effects()).expect("loop");
        assert_eq!(events.0.len(), 3, "the loop reads no event after the stop flag");
        assert_eq!(canvas.draws, 0);
    }

    #[test]
    fn stop_flag_set_during_wait_ends_loop() {
        /// Sets the stop flag in the wait, as a signal does.
        struct Signal<'a>(&'a AtomicBool);
        impl EventSource for Signal<'_> {
            fn next(&mut self, _timeout: Duration) -> io::Result<Option<Event>> {
                self.0.store(true, Ordering::SeqCst);
                Ok(None)
            }
        }
        let stop = AtomicBool::new(false);
        let mut canvas = Counting::new();
        let (_tx, rx) = mpsc::channel();
        run_loop(&mut app(), &mut Signal(&stop), &mut canvas, &stop, &rx, &mut no_effects()).expect("loop");
        assert_eq!(canvas.draws, 0, "a signal stops the loop before the next draw");
    }

    #[test]
    fn read_error_ends_loop_cleanly() {
        let mut events = script(vec![Err(io::Error::from_raw_os_error(5))]);
        let mut canvas = Counting::new();
        let (_tx, rx) = mpsc::channel();
        let stop = AtomicBool::new(false);
        let result = run_loop(&mut app(), &mut events, &mut canvas, &stop, &rx, &mut no_effects());
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn quit_effect_ends_loop() {
        let mut events = script(vec![key('q'), Ok(None), Ok(None)]);
        let mut canvas = Counting::new();
        let (_tx, rx) = mpsc::channel();
        let stop = AtomicBool::new(false);
        run_loop(&mut app(), &mut events, &mut canvas, &stop, &rx, &mut no_effects()).expect("loop");
        assert_eq!(events.0.len(), 2, "the loop stops at q");
    }

    #[test]
    fn other_effects_reach_the_caller() {
        let mut events = script(vec![key('z'), Ok(None), key('r'), Ok(None)]);
        let mut canvas = Counting::new();
        let (_tx, rx) = mpsc::channel();
        let stop = AtomicBool::new(false);
        let mut seen = Vec::new();
        let mut effects = |effect, _: &mut App| seen.push(effect);
        run_loop(&mut app(), &mut events, &mut canvas, &stop, &rx, &mut effects).expect("loop");
        assert_eq!(seen, [Effect::ToggleZoom, Effect::Reload]);
        assert_eq!(canvas.draws, 2);
    }

    #[test]
    fn state_update_reaches_header() {
        let mut events = script(vec![Ok(None)]);
        let mut canvas = Counting::new();
        let (tx, rx) = mpsc::channel();
        tx.send(AgentState {
            agent: Some(Agent::Pi),
            safety: Safety::Clear { state: Some("working".into()) },
        })
        .expect("send");
        let stop = AtomicBool::new(false);
        run_loop(&mut app(), &mut events, &mut canvas, &stop, &rx, &mut no_effects()).expect("loop");
        assert!(canvas.header().contains("pi (working…)"), "{}", canvas.header());
    }
}
