//! The terminal loop of the viewer and the `annorex last` command.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, SystemTime};

use annorex_hosts::roots::Roots;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};

use crate::agent::{Agent, NOT_AGENT, detect};
use crate::clipboard::{SystemClipboard, copy_review};
use crate::config::Config;
use crate::deliver::{DeliveryMode, RealSleeper};
use crate::draft::{DraftError, DraftKey, DraftStore};
use crate::files;
use crate::replies::{
    Candidate, Reply, SessionRef, Source, candidates, load_replies, matching_panes, needs_picker,
    process_table,
};
use crate::rex::{Rex, RexError, Runner, SystemRunner};
use crate::safety::{Safety, check};
use crate::send::{apply_result, send_review};
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
            let effect = match event {
                Event::Key(key) => Some(app.handle_key(key)),
                Event::Paste(text) => Some(app.handle_paste(&text)),
                _ => None,
            };
            if let Some(effect) = effect {
                match effect {
                    Effect::None => {}
                    Effect::Quit => return Ok(()),
                    effect => {
                        effects(effect, app);
                        if app.should_exit() {
                            return Ok(());
                        }
                    }
                }
            }
            let Ok(next) = events.next(Duration::ZERO) else { return Ok(()) };
            pending = next;
        }
        while let Ok(state) = updates.try_recv() {
            app.set_state(state);
        }
        app.poll_background();
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

/// After the viewer quits, focus the agent. A viewer outside a Rex layer has no `REX_BLOCK` and does nothing.
fn hand_back_focus(rex: &Rex<SystemRunner>, target: &str) {
    if let Some(own) = std::env::var_os("REX_BLOCK").and_then(|value| value.into_string().ok()) {
        crate::open::release_focus(rex, &own, target);
    }
}

/// `annorex last`: show the replies of the agent in `target` in this terminal. Every problem after
/// the Rex discovery shows in the pane, so the user can read it.
pub fn run_last(
    rex: &Rex<SystemRunner>,
    target: &str,
    config: &Config,
    store: Option<DraftStore>,
) -> io::Result<()> {
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
        store,
        enter_delay: config.enter_delay,
        files: None,
    };
    let (screen, replies) = viewer.start_or_error();
    let mut app = App::new(viewer.state.clone(), screen, replies, config);
    if viewer.store.is_none() {
        app.set_save_error(Some("no home directory, so no place for drafts".to_owned()));
    }
    viewer.restore_drafts(&mut app);

    let (sender, updates) = mpsc::channel();
    let watcher = Rex::new(rex.bin.clone(), rex.session.clone(), SystemRunner);
    let pane = target.to_owned();
    std::thread::spawn(move || watch_status(&watcher, &pane, &sender));

    let mut terminal = start_terminal(init_terminal, restore_terminal)?;
    let result = run_loop(
        &mut app,
        &mut CrosstermEvents,
        &mut terminal,
        &stop,
        &updates,
        &mut |effect, app: &mut App| viewer.apply(effect, app),
    );
    restore_terminal();
    hand_back_focus(rex, target);
    result
}

/// `annorex files`: review the files of the work directory of the agent in `target`. `paths` open
/// at the start. Every problem shows in the pane, so the user can read it.
pub fn run_files(
    rex: &Rex<SystemRunner>,
    target: &str,
    paths: Vec<PathBuf>,
    config: &Config,
    store: Option<DraftStore>,
) -> io::Result<()> {
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
        store,
        enter_delay: config.enter_delay,
        files: Some(FilesView { config: config.clone(), paths, session: None }),
    };
    let mut app = match viewer.start_files() {
        Ok(root) => viewer.files_app(&root),
        Err(message) => App::new(viewer.state.clone(), Screen::Error(message), Vec::new(), config),
    };
    app.set_state(viewer.state.clone());
    if viewer.store.is_none() {
        app.set_save_error(Some("no home directory, so no place for drafts".to_owned()));
    }
    let (sender, updates) = mpsc::channel();
    let watcher = Rex::new(rex.bin.clone(), rex.session.clone(), SystemRunner);
    let pane = target.to_owned();
    std::thread::spawn(move || watch_status(&watcher, &pane, &sender));

    let mut terminal = start_terminal(init_terminal, restore_terminal)?;
    let result = run_loop(
        &mut app,
        &mut CrosstermEvents,
        &mut terminal,
        &stop,
        &updates,
        &mut |effect, app: &mut App| viewer.apply(effect, app),
    );
    restore_terminal();
    hand_back_focus(rex, target);
    result
}

/// Raw mode, the alternate screen, and bracketed paste, so a paste is one event and not keys.
fn init_terminal() -> io::Result<DefaultTerminal> {
    let terminal = ratatui::try_init()?;
    ratatui::crossterm::execute!(io::stdout(), event::EnableBracketedPaste)?;
    Ok(terminal)
}

fn restore_terminal() {
    let _ = ratatui::crossterm::execute!(io::stdout(), event::DisableBracketedPaste);
    ratatui::restore();
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
    /// Where the drafts are. `None` when there is no home directory.
    store: Option<DraftStore>,
    /// The wait between the paste and the Enter key.
    enter_delay: Duration,
    /// Set in the files viewer.
    files: Option<FilesView>,
}

/// What the files viewer needs besides the session viewer.
struct FilesView {
    config: Config,
    /// The files to open at the start.
    paths: Vec<PathBuf>,
    /// The key of the drafts: the work directory of the agent. Notes on files belong to the
    /// directory, not to one session of the agent.
    session: Option<String>,
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

    /// The session part of a draft key: the work directory in the files viewer, else the session.
    fn draft_session(&self) -> Option<String> {
        match &self.files {
            Some(files) => files.session.clone(),
            None => self.session.as_ref().map(SessionRef::id),
        }
    }

    /// The key of the draft of a reply in the session on the screen.
    fn draft_key(&self, reply_id: &str) -> Option<DraftKey> {
        Some(DraftKey {
            agent: self.state.agent?,
            session: self.draft_session()?,
            document: reply_id.to_owned(),
        })
    }

    /// Load the draft of each reply that the app has no review for. A draft that cannot be read
    /// stays on disk and shows in the footer.
    fn restore_drafts(&self, app: &mut App) {
        let Some(store) = &self.store else { return };
        for id in app.reply_ids() {
            if app.has_review(&id) {
                continue;
            }
            let Some(key) = self.draft_key(&id) else { return };
            match store.load(&key) {
                Ok(Some(draft)) => app.restore(&id, draft),
                Ok(None) => {}
                Err(error @ DraftError::Unreadable { .. }) => app.set_status(error.to_string()),
                Err(error) => app.set_save_error(Some(error.to_string())),
            }
        }
    }

    /// Load the draft of one document.
    fn restore_one(&self, app: &mut App, id: &str) {
        let (Some(store), Some(key)) = (&self.store, self.draft_key(id)) else { return };
        match store.load(&key) {
            Ok(Some(draft)) => app.restore(id, draft),
            Ok(None) => {}
            Err(error @ DraftError::Unreadable { .. }) => app.set_status(error.to_string()),
            Err(error) => app.set_save_error(Some(error.to_string())),
        }
    }

    /// Find the agent and the work directory of the files viewer.
    fn start_files(&mut self) -> Result<PathBuf, String> {
        self.state = read_state(self.rex, self.target).map_err(|error| error.to_string())?;
        let Some(agent) = self.state.agent else { return Err(NOT_AGENT.to_owned()) };
        let process = self.rex.process(self.target).map_err(|error| error.to_string())?;
        let source = Source::from_process(agent, &process)
            .ok_or_else(|| "Rex does not report the directory of the agent process".to_owned())?;
        if let Some(files) = &mut self.files {
            files.session = Some(format!("files:{}", source.cwd.display()));
        }
        Ok(source.cwd)
    }

    /// The viewer of the files of `root`: the files of the command line open, the tree starts when
    /// none did, and the marks show the files with unsent drafts.
    fn files_app(&mut self, root: &Path) -> App {
        let (config, paths) = self.files.as_ref().map_or_else(
            || (Config::default(), Vec::new()),
            |files| (files.config.clone(), files.paths.clone()),
        );
        let mut app = App::new_files(self.state.clone(), root.to_path_buf(), &config);
        app.start_scan();
        app.start_watch();
        let mut problems: Vec<String> = Vec::new();
        for input in paths {
            match files::resolve(root, &input) {
                // A directory opens the tree there.
                Ok(relative) if root.join(&relative).is_dir() => app.reveal_dir(&relative),
                Ok(relative) => match files::load(root, &relative) {
                    Ok(loaded) => {
                        app.open_file(loaded);
                    }
                    Err(error) => problems.push(error.to_string()),
                },
                Err(error) => problems.push(error.to_string()),
            }
        }
        self.restore_drafts(&mut app);
        self.refresh_marks(&mut app);
        if app.reply_ids().is_empty() {
            app.open_tree();
        }
        if !problems.is_empty() {
            app.set_status(problems.join("; "));
        }
        app
    }

    /// Tell the app which files have a draft with notes that no send took.
    fn refresh_marks(&self, app: &mut App) {
        if let (Some(store), Some(agent), Some(session)) =
            (&self.store, self.state.agent, self.draft_session())
        {
            app.set_draft_marks(&store.unsent(agent, &session));
        }
    }

    /// `r` on the error screen of the files viewer: look for the agent again.
    fn restart_files(&mut self, app: &mut App) {
        match self.start_files() {
            Ok(root) => *app = self.files_app(&root),
            Err(message) => app.set_error(message),
        }
        app.set_state(self.state.clone());
    }

    /// Give the export of the reply on the screen to the agent, and save the draft when the notes
    /// count as sent.
    fn send(&self, app: &mut App, mode: DeliveryMode) {
        let export = app.export();
        let result = send_review(self.rex, &RealSleeper, self.target, &export, mode, self.enter_delay);
        if apply_result(app, result, &crate::draft::now()) {
            self.save_drafts(app);
            self.refresh_marks(app);
        }
    }

    /// The review is gone from the app. Remove the draft file too.
    fn discard(&self, app: &mut App) {
        for id in app.take_discarded() {
            if let (Some(store), Some(key)) = (&self.store, self.draft_key(&id))
                && let Err(error) = store.discard(&key)
            {
                app.set_save_error(Some(error.to_string()));
            }
        }
        self.refresh_marks(app);
        app.quit_if_pending();
    }

    /// Save the draft of the reply or the file on the screen.
    fn save_draft(&self, app: &mut App) {
        let (Some(store), Some(agent), Some(session)) = (&self.store, self.state.agent, self.draft_session())
        else {
            return;
        };
        let error = app.draft(agent, &session).and_then(|draft| store.save(&draft).err());
        app.set_save_error(error.map(|error| error.to_string()));
    }

    /// Save the drafts of all open files: after a send, every file changed its sent state.
    fn save_drafts(&self, app: &mut App) {
        let (Some(store), Some(agent), Some(session)) = (&self.store, self.state.agent, self.draft_session())
        else {
            return;
        };
        let error = app.drafts(agent, &session).iter().find_map(|draft| store.save(draft).err());
        app.set_save_error(error.map(|error| error.to_string()));
    }

    fn apply(&mut self, effect: Effect, app: &mut App) {
        match effect {
            Effect::None | Effect::Quit => {}
            Effect::SaveDraft => self.save_draft(app),
            Effect::Send(mode) => self.send(app, mode),
            Effect::SendWithNote => {
                self.save_draft(app);
                self.send(app, DeliveryMode::Send);
            }
            Effect::Copy => copy_review(app, &mut SystemClipboard::new()),
            Effect::Discard => self.discard(app),
            Effect::FileOpened => {
                if let Some(id) = app.shown_reply_id().map(str::to_owned) {
                    self.restore_one(app, &id);
                }
                self.refresh_marks(app);
            }
            Effect::Reload => self.reload(app),
            Effect::PickSession(index) => {
                let (screen, replies) = self.open(index);
                show(app, screen, replies);
                self.restore_drafts(app);
            }
        }
    }

    /// Read the replies again. Claude Code finds its session again by pid, because the session
    /// file can change while the agent runs. Pi and `OpenCode` keep the session that the user saw.
    fn reload(&mut self, app: &mut App) {
        if self.files.is_some() {
            self.restart_files(app);
            return;
        }
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
                self.restore_drafts(app);
            }
            Err(error) if showing => app.set_status(error.to_string()),
            Err(error) => app.set_error(error.to_string()),
        }
        app.set_state(self.state.clone());
    }
}

/// Put a new screen in the app. Replies go through `set_replies`, which keeps the selected reply.
fn show(app: &mut App, screen: Screen, replies: Vec<Reply>) {
    match screen {
        Screen::Viewer => app.set_replies(replies),
        Screen::Error(message) => app.set_error(message),
        Screen::Picker(candidates) => app.set_picker(candidates),
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
    use crate::config::Config;
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
        App::new(state, Screen::Viewer, vec![reply], &Config::default())
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
    fn a_send_from_the_quit_question_ends_the_loop() {
        use crate::send::{SendResult, apply_result};
        let enter = Ok(Some(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))));
        let mut events = script(vec![key('c'), key('x'), enter, key('q'), key('s'), key('j'), Ok(None)]);
        let mut canvas = Counting::new();
        let (_tx, rx) = mpsc::channel();
        let stop = AtomicBool::new(false);
        let mut app = app();
        let mut sent = 0;
        let mut effects = |effect: Effect, app: &mut App| {
            if matches!(effect, Effect::Send(_)) {
                sent += 1;
                apply_result(app, SendResult::Sent, "2026-11-09T14:30:00Z");
            }
        };
        run_loop(&mut app, &mut events, &mut canvas, &stop, &rx, &mut effects).expect("loop");
        assert_eq!(sent, 1);
        assert_eq!(canvas.draws, 0, "the loop ended in the same turn as the send");
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
        let mut events = script(vec![key('r'), Ok(None), key('r'), Ok(None)]);
        let mut canvas = Counting::new();
        let (_tx, rx) = mpsc::channel();
        let stop = AtomicBool::new(false);
        let mut seen = Vec::new();
        let mut effects = |effect, _: &mut App| seen.push(effect);
        run_loop(&mut app(), &mut events, &mut canvas, &stop, &rx, &mut effects).expect("loop");
        assert_eq!(seen, [Effect::Reload, Effect::Reload]);
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
