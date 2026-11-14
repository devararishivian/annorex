//! The terminal loop of the viewer and the `annorex last` command.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, SystemTime};

use annorex_hosts::roots::Roots;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::terminal::supports_keyboard_enhancement;
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
use crate::send::{SendResult, apply_result, save_after_send, send_review};
use crate::ui::{AgentState, App, Effect, ReplySource, Screen, StartView, draw};

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

/// True when an event can change the screen. A pointer move does nothing in the viewer.
fn needs_draw(event: &Event) -> bool {
    !matches!(event, Event::Mouse(mouse) if mouse.kind == event::MouseEventKind::Moved)
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
        // A timeout draws, so the clock and the background work show. A batch of pointer moves alone does not.
        let mut redraw = pending.is_none();
        // True when an earlier event of this batch changed the state, so the hit map is old.
        let mut stale = false;
        while let Some(event) = pending {
            if stale
                && matches!(&event, Event::Mouse(mouse) if matches!(mouse.kind, event::MouseEventKind::Down(_)))
            {
                canvas.draw(app)?;
                stale = false;
            }
            redraw |= needs_draw(&event);
            stale |= needs_draw(&event);
            let effect = match event {
                Event::Key(key) => Some(app.handle_key(key)),
                Event::Paste(text) => Some(app.handle_paste(&text)),
                Event::Mouse(mouse) => Some(app.handle_mouse(mouse)),
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
            redraw = true;
        }
        redraw |= app.poll_background();
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        if redraw {
            canvas.draw(app)?;
        }
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

/// Which document the viewer shows first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// The newest reply of the agent.
    Last,
    /// The files of the command line. With none, the file tree opens.
    Files(Vec<PathBuf>),
}

/// `annorex last` and `annorex files`: review the replies of the agent in `target` and the files of its
/// work directory in this terminal. Every problem after the Rex discovery shows in the pane, so the user
/// can read it.
pub fn run_viewer(
    rex: &Rex<SystemRunner>,
    target: &str,
    start: &Start,
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
        config: config.clone(),
        work: None,
        start: start.clone(),
    };
    let mut app = viewer.start_app(start);
    app.set_state(viewer.state.clone());
    let (sender, updates) = mpsc::channel();
    let watcher = Rex::new(rex.bin.clone(), rex.session.clone(), SystemRunner);
    let pane = target.to_owned();
    std::thread::spawn(move || watch_status(&watcher, &pane, &sender));

    let (mut terminal, enhanced) = start_terminal(init_terminal, restore_terminal)?;
    app.shift_enter = enhanced;
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

/// True while the keyboard enhancement flags of this process are on in the terminal.
static ENHANCED: AtomicBool = AtomicBool::new(false);

/// Raw mode, the alternate screen, bracketed paste (so a paste is one event and not keys), and the
/// mouse. When the terminal supports it, also the keyboard enhancement that makes `shift+enter` a
/// key of its own [src: crossterm 0.29.0]. The bool is true when that is on. A panic turns the
/// paste mode, the mouse, and the enhancement off before the panic hook of ratatui restores the rest.
fn init_terminal() -> io::Result<(DefaultTerminal, bool)> {
    static PANIC_HOOK: std::sync::Once = std::sync::Once::new();
    let terminal = ratatui::try_init()?;
    // `try_init` adds its hook at each call. The hook of this function goes in one time only, so a
    // second viewer in one process does not stack a second copy.
    PANIC_HOOK.call_once(|| {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            stop_paste_and_mouse();
            hook(info);
        }));
    });
    ratatui::crossterm::execute!(io::stdout(), event::EnableBracketedPaste, event::EnableMouseCapture)?;
    let enhanced = supports_keyboard_enhancement().unwrap_or(false);
    if enhanced {
        // Set the flag first: a failed push still gets one pop, which a terminal ignores.
        ENHANCED.store(true, Ordering::SeqCst);
        ratatui::crossterm::execute!(
            io::stdout(),
            event::PushKeyboardEnhancementFlags(event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    Ok((terminal, enhanced))
}

fn stop_paste_and_mouse() {
    let _ =
        ratatui::crossterm::execute!(io::stdout(), event::DisableMouseCapture, event::DisableBracketedPaste);
    if ENHANCED.swap(false, Ordering::SeqCst) {
        let _ = ratatui::crossterm::execute!(io::stdout(), event::PopKeyboardEnhancementFlags);
    }
}

fn restore_terminal() {
    stop_paste_and_mouse();
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
    config: Config,
    /// The agent process and its directory. `None` before the first start or when there is none.
    work: Option<Source>,
    /// What the viewer was asked to show first. A new start after an error uses it again.
    start: Start,
}

/// The `session` part of the draft key of a file: the notes belong to the work directory, not to one
/// session of the agent.
fn files_session(source: &Source) -> String {
    format!("files:{}", source.cwd.display())
}

impl<R: Runner> Viewer<'_, R> {
    /// Find the agent, its work directory, and its replies, and build the app. Only a missing agent or
    /// work directory gives a message in place of the viewer. Other problems show in the reply list.
    fn start_app(&mut self, start: &Start) -> App {
        self.start = start.clone();
        if let Some(store) = &self.store {
            // Drafts that a send used up are gone from the disk before the first read.
            store.prune_sent();
        }
        let mut app = self.build_app(start);
        if self.store.is_none() {
            app.set_save_error(Some("no home directory, so no place for drafts".to_owned()));
        }
        app
    }

    fn build_app(&mut self, start: &Start) -> App {
        let source = match self.locate() {
            Ok(source) => source,
            Err(message) => {
                return App::new(self.state.clone(), Screen::Error(message), Vec::new(), &self.config);
            }
        };
        let root = source.cwd.clone();
        let mut app = App::new(self.state.clone(), Screen::Viewer, Vec::new(), &self.config)
            .with_root(root.clone())
            .with_start_view(start_view(start));
        app.start_scan();
        app.start_watch();
        let turns = self.config.changes_turns;
        app.start_changes(crate::changes::looker(source, Roots::from_env(), root.clone(), turns));
        let (reply_source, replies) = self.find_replies();
        show(&mut app, reply_source, replies);
        app.open_start_list();
        self.watch_replies(&mut app);
        let mut problems: Vec<String> = Vec::new();
        if let Start::Files(paths) = start {
            for input in paths {
                match files::resolve(&root, input) {
                    // A directory opens the tree there.
                    Ok(relative) if root.join(&relative).is_dir() => app.reveal_dir(&relative),
                    Ok(relative) => match files::load(&root, &relative) {
                        Ok(loaded) => {
                            app.open_file(loaded);
                        }
                        Err(error) => problems.push(error.to_string()),
                    },
                    Err(error) => problems.push(error.to_string()),
                }
            }
        }
        hide_reply_for_files(start, &mut app);
        self.restore_drafts(&mut app);
        self.refresh_marks(&mut app);
        if opens_tree(start, &app) {
            app.open_tree();
        }
        if !problems.is_empty() {
            app.set_status(problems.join("; "));
        }
        app
    }

    /// Watch the transcript for a new reply. Claude Code finds its session again by pid. Pi and `OpenCode` keep the
    /// session that the user saw. While the user must choose a session, nothing is watched.
    fn watch_replies(&self, app: &mut App) {
        let Some(source) = self.work.clone() else { return };
        if matches!(app.reply_source(), ReplySource::Pick(_)) {
            app.stop_reply_poll();
            return;
        }
        let fixed = if self.state.agent == Some(Agent::ClaudeCode) { None } else { self.session.clone() };
        app.start_reply_poll(crate::news::reply_looker(source, Roots::from_env(), fixed));
    }

    /// Read the agent and its state, and find its process and work directory.
    fn locate(&mut self) -> Result<Source, String> {
        // A failed look must not clear what the viewer already knows, or the drafts could not be saved.
        let state = read_state(self.rex, self.target).map_err(|error| error.to_string())?;
        let Some(agent) = state.agent else {
            if self.work.is_none() {
                self.state = state;
            }
            return Err(NOT_AGENT.to_owned());
        };
        let process = self.rex.process(self.target).map_err(|error| error.to_string())?;
        let source = Source::from_process(agent, &process)
            .ok_or_else(|| "Rex does not report the directory of the agent process".to_owned())?;
        self.state = state;
        self.work = Some(source.clone());
        Ok(source)
    }

    /// Find the session of the agent and read its replies. A problem is a [`ReplySource`], not an error.
    fn find_replies(&mut self) -> (ReplySource, Vec<Reply>) {
        let missing = |message: String| (ReplySource::Missing(message), Vec::new());
        let (Some(source), Some(agent)) = (self.work.clone(), self.state.agent) else {
            return missing(NOT_AGENT.to_owned());
        };
        let table = match process_table() {
            Ok(table) => table,
            Err(error) => return missing(format!("cannot read the process table: {error}")),
        };
        let found = match candidates(&source, &Roots::from_env(), &table) {
            Ok(found) => found,
            Err(error) => return missing(error.to_string()),
        };
        let panes = if agent == Agent::ClaudeCode {
            1
        } else {
            match matching_panes(self.rex, agent, &source.cwd) {
                Ok(panes) => panes,
                Err(error) => return missing(error.to_string()),
            }
        };
        self.candidates = found;
        if needs_picker(agent, panes, &self.candidates, SystemTime::now()) {
            return (ReplySource::Pick(self.candidates.clone()), Vec::new());
        }
        self.open(0)
    }

    /// Read the replies of candidate `index`.
    fn open(&mut self, index: usize) -> (ReplySource, Vec<Reply>) {
        let Some(candidate) = self.candidates.get(index) else {
            return (
                ReplySource::Missing("the session is gone; press r to look again".to_owned()),
                Vec::new(),
            );
        };
        self.session = Some(candidate.session.clone());
        read_session(&candidate.session)
    }

    /// The session part of the draft key of a document: the work directory for a file, else the
    /// session of the replies.
    fn session_of(&self, id: &str) -> Option<String> {
        if id.starts_with("file:") {
            self.work.as_ref().map(files_session)
        } else {
            self.session.as_ref().map(SessionRef::id)
        }
    }

    /// The key of the draft of a document.
    fn draft_key(&self, id: &str) -> Option<DraftKey> {
        Some(DraftKey { agent: self.state.agent?, session: self.session_of(id)?, document: id.to_owned() })
    }

    /// Load the draft of each document that the app has no review for. A draft that cannot be read
    /// stays on disk and shows in the footer.
    fn restore_drafts(&self, app: &mut App) {
        for id in app.reply_ids() {
            if !app.has_review(&id) {
                self.restore_one(app, &id);
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

    /// Tell the app which files have a draft with notes that no send took.
    fn refresh_marks(&self, app: &mut App) {
        if let (Some(store), Some(agent), Some(source)) = (&self.store, self.state.agent, &self.work) {
            app.set_draft_marks(&store.unsent(agent, &files_session(source)));
        }
    }

    /// Give the export of the app to the agent, and save the drafts when the notes count as sent.
    fn send(&self, app: &mut App, mode: DeliveryMode) {
        let export = app.export();
        let result = send_review(self.rex, &RealSleeper, self.target, &export, mode, self.enter_delay);
        if apply_result(app, result.clone(), &crate::draft::now()) {
            self.save_drafts(app, &result);
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

    /// Save the draft of the document on the screen.
    fn save_draft(&self, app: &mut App) {
        let (Some(store), Some(agent), Some(id)) =
            (&self.store, self.state.agent, app.shown_reply_id().map(str::to_owned))
        else {
            return;
        };
        let Some(session) = self.session_of(&id) else {
            if app.review().is_some_and(|review| !review.is_empty()) {
                app.set_save_error(Some(
                    "no session for this document, so the draft cannot be saved".to_owned(),
                ));
            }
            return;
        };
        let error = app.draft(agent, &session).and_then(|draft| store.save(&draft).err());
        app.set_save_error(error.map(|error| error.to_string()));
    }

    /// Save the drafts of all documents after a send. A finished send clears what it took. A paste
    /// or a held Enter keeps the notes with their sent marks.
    fn save_drafts(&self, app: &mut App, result: &SendResult) {
        let (Some(store), Some(agent)) = (&self.store, self.state.agent) else { return };
        let drafts = app.drafts(agent, |id| self.session_of(id));
        let error = save_after_send(store, &drafts, result);
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
                let (reply_source, replies) = self.open(index);
                show(app, reply_source, replies);
                self.watch_replies(app);
                self.restore_drafts(app);
            }
        }
    }

    /// Read the replies again. Claude Code finds its session again by pid, because the session
    /// file can change while the agent runs. Pi and `OpenCode` keep the session that the user saw.
    fn reload(&mut self, app: &mut App) {
        if matches!(app.screen, Screen::Error(_)) {
            // The start found no agent or no directory. Look again.
            let start = self.start.clone();
            *app = self.start_app(&start);
            app.set_state(self.state.clone());
            return;
        }
        let showing = *app.reply_source() == ReplySource::Ready;
        let result = match &self.session {
            Some(session) if self.state.agent != Some(Agent::ClaudeCode) => read_session(session),
            _ => {
                if let Err(message) = self.locate() {
                    app.set_status(message);
                    return;
                }
                self.find_replies()
            }
        };
        match result {
            // Keep the reply on the screen. The footer tells why the reload failed.
            (ReplySource::Missing(message), _) if showing => app.set_status(message),
            (reply_source, replies) => {
                app.cache.clear();
                show(app, reply_source, replies);
                self.watch_replies(app);
                self.restore_drafts(app);
            }
        }
        app.set_state(self.state.clone());
    }
}

/// The view of the viewer for `start`: `last` shows a reply, and `files` shows the files.
fn start_view(start: &Start) -> StartView {
    match start {
        Start::Last => StartView::Last,
        Start::Files(_) => StartView::Files,
    }
}

/// True when the viewer starts on the file tree: with no document, or for `files` with no path. A choice of
/// sessions that `last` shows stays on the screen.
fn opens_tree(start: &Start, app: &App) -> bool {
    let no_paths = matches!(start, Start::Files(paths) if paths.is_empty());
    let picking = matches!(start, Start::Last) && matches!(app.reply_source(), ReplySource::Pick(_));
    no_paths || (app.reply_ids().is_empty() && !picking)
}

/// The files view shows no document until a file is open: a reply stays behind the list.
fn hide_reply_for_files(start: &Start, app: &mut App) {
    if matches!(start, Start::Files(_)) && app.shown_file().is_none() {
        app.show_no_document();
    }
}

/// Read the replies of a session.
fn read_session(session: &SessionRef) -> (ReplySource, Vec<Reply>) {
    match load_replies(session) {
        Ok(replies) => (ReplySource::Ready, replies),
        Err(error) => (ReplySource::Missing(error.to_string()), Vec::new()),
    }
}

/// Put the replies, or the reason for none, in the app. Replies go through `set_replies`, which
/// keeps the selected document.
fn show(app: &mut App, source: ReplySource, replies: Vec<Reply>) {
    match source {
        ReplySource::Ready => app.set_replies(replies),
        other => {
            app.set_replies(Vec::new());
            app.set_reply_source(other);
        }
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
    fn pointer_moves_alone_cost_no_draw() {
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let mouse = |kind| {
            Ok(Some(Event::Mouse(MouseEvent { kind, column: 1, row: 1, modifiers: KeyModifiers::NONE })))
        };
        let moved = || mouse(MouseEventKind::Moved);
        let mut events = script(vec![moved(), moved(), moved(), Ok(None)]);
        let mut canvas = Counting::new();
        let (_tx, rx) = mpsc::channel();
        let stop = AtomicBool::new(false);
        run_loop(&mut app(), &mut events, &mut canvas, &stop, &rx, &mut no_effects()).expect("loop");
        assert_eq!(canvas.draws, 0, "a batch of pointer moves changes nothing on the screen");
        let mut events =
            script(vec![moved(), mouse(MouseEventKind::Up(MouseButton::Left)), Ok(None), Ok(None)]);
        let mut canvas = Counting::new();
        run_loop(&mut app(), &mut events, &mut canvas, &stop, &rx, &mut no_effects()).expect("loop");
        assert_eq!(canvas.draws, 2, "another event and the timeout each draw");
        assert!(super::needs_draw(&Event::Resize(80, 24)));
        assert!(!super::needs_draw(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE
        })));
    }

    #[test]
    fn a_click_after_a_key_in_one_batch_acts_on_the_drawn_row() {
        use crate::ui::Mode;
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let text = (0..200).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n\n");
        let reply = Reply { id: "r1".into(), text, at: None };
        let state = AgentState { agent: None, safety: Safety::Clear { state: None } };
        let mut app = App::new(state, Screen::Viewer, vec![reply], &Config::default());
        let mut canvas = Counting::new();
        let (_tx, rx) = mpsc::channel();
        let stop = AtomicBool::new(false);
        // The first draw shows the top of the document. Block 1 is on the row of "line 1".
        let mut first = script(vec![Ok(None)]);
        run_loop(&mut app, &mut first, &mut canvas, &stop, &rx, &mut no_effects()).expect("loop");
        let buffer = canvas.terminal.backend().buffer().clone();
        let row = (0..24u16)
            .find(|&y| {
                (0..80u16)
                    .map(|x| buffer.cell((x, y)).map_or(" ", |c| c.symbol()))
                    .collect::<String>()
                    .contains("line 1 ")
            })
            .expect("row of block 1");
        // Blocks have a blank row between them, so the same row may be blank in the new frame.
        // Click on this row and on the next one: one of them is a block in the new frame.
        let click = |row| {
            Ok(Some(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 0,
                row,
                modifiers: KeyModifiers::NONE,
            })))
        };
        let mut batch = script(vec![key('G'), click(row), click(row + 1), Ok(None)]);
        run_loop(&mut app, &mut batch, &mut canvas, &stop, &rx, &mut no_effects()).expect("loop");
        match app.mode {
            Mode::Block(index) => {
                assert!(index > 100, "the click hit the stale row of block 1: block {index}");
            }
            other => panic!("unexpected mode {other:?}"),
        }
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
        let mut events =
            script(vec![key('j'), key('c'), key('x'), enter, key('q'), key('s'), key('j'), Ok(None)]);
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
        assert!(canvas.header().contains("Pi (working…)"), "{}", canvas.header());
    }
    /// A runner that is never called: the draft keys need no Rex call.
    struct NoRex;

    impl crate::rex::Runner for NoRex {
        fn run(
            &self,
            _argv: &[std::ffi::OsString],
            _stdin: Option<&[u8]>,
        ) -> io::Result<std::process::Output> {
            Err(io::Error::from_raw_os_error(5))
        }
    }

    fn viewer(
        rex: &crate::rex::Rex<NoRex>,
        work: Option<crate::replies::Source>,
    ) -> super::Viewer<'_, NoRex> {
        super::Viewer {
            rex,
            target: "block:b1",
            state: AgentState { agent: Some(Agent::ClaudeCode), safety: Safety::Clear { state: None } },
            session: Some(crate::replies::SessionRef::Pi(std::path::PathBuf::from("/sessions/one.jsonl"))),
            candidates: Vec::new(),
            store: None,
            enter_delay: Duration::ZERO,
            config: Config::default(),
            work,
            start: super::Start::Last,
        }
    }

    fn source() -> crate::replies::Source {
        crate::replies::Source { agent: Agent::ClaudeCode, cwd: std::path::PathBuf::from("/work"), pid: 7 }
    }

    #[test]
    fn the_draft_key_of_a_reply_uses_its_session_and_of_a_file_the_directory() {
        let rex = crate::rex::Rex::new(std::path::PathBuf::from("rex"), "session:s".to_owned(), NoRex);
        let viewer = viewer(&rex, Some(source()));
        let reply = viewer.draft_key("r1").expect("reply key");
        let file = viewer.draft_key("file:a.md").expect("file key");
        assert_eq!((reply.document.as_str(), reply.session.as_str()), ("r1", "one"));
        assert_eq!((file.document.as_str(), file.session.as_str()), ("file:a.md", "files:/work"));
    }

    #[test]
    fn a_file_has_a_draft_key_without_a_reply_session() {
        let rex = crate::rex::Rex::new(std::path::PathBuf::from("rex"), "session:s".to_owned(), NoRex);
        let mut viewer = viewer(&rex, Some(source()));
        viewer.session = None;
        assert!(viewer.draft_key("r1").is_none(), "a reply needs its session");
        assert!(viewer.draft_key("file:a.md").is_some(), "a file needs only the directory");
    }
    #[test]
    fn a_failed_look_keeps_the_known_agent_and_directory() {
        let rex = crate::rex::Rex::new(std::path::PathBuf::from("rex"), "session:s".to_owned(), NoRex);
        let mut viewer = viewer(&rex, Some(source()));
        assert!(viewer.locate().is_err(), "the runner always fails");
        assert!(viewer.work.is_some() && viewer.state.agent.is_some(), "the drafts need them");
        assert!(viewer.draft_key("file:a.md").is_some());
    }

    #[test]
    fn last_with_a_choice_of_sessions_keeps_the_choice_and_files_open_the_tree() {
        let candidate = crate::replies::Candidate {
            session: crate::replies::SessionRef::Pi(std::path::PathBuf::from("/s.jsonl")),
            updated: std::time::SystemTime::now(),
            last_line: "x".to_owned(),
        };
        let mut app = App::new(
            AgentState { agent: Some(Agent::Pi), safety: Safety::Clear { state: None } },
            Screen::Viewer,
            Vec::new(),
            &Config::default(),
        )
        .with_root(std::path::PathBuf::from("/work"));
        app.set_reply_source(crate::ui::ReplySource::Pick(vec![candidate]));
        assert!(!super::opens_tree(&super::Start::Last, &app));
        assert!(super::opens_tree(&super::Start::Files(Vec::new()), &app));
        let empty = App::new(
            AgentState { agent: Some(Agent::Pi), safety: Safety::Clear { state: None } },
            Screen::Viewer,
            Vec::new(),
            &Config::default(),
        );
        assert!(super::opens_tree(&super::Start::Last, &empty), "no document at all: the tree");
    }

    #[test]
    fn last_starts_the_last_reply_view_and_files_the_files_view() {
        use crate::ui::StartView;
        let mut app = App::new(
            AgentState { agent: Some(Agent::Pi), safety: Safety::Clear { state: None } },
            Screen::Viewer,
            Vec::new(),
            &Config::default(),
        )
        .with_root(std::path::PathBuf::from("/work"));
        app.set_replies(vec![Reply { id: "r".into(), text: "hi\n".into(), at: None }]);
        super::hide_reply_for_files(&super::Start::Last, &mut app);
        assert!(app.shown_reply_id().is_some(), "the last view keeps its reply");
        super::hide_reply_for_files(&super::Start::Files(Vec::new()), &mut app);
        assert!(app.shown_reply_id().is_none(), "the files view with no file shows no document");
        assert_eq!(super::start_view(&super::Start::Last), StartView::Last);
        assert_eq!(super::start_view(&super::Start::Files(Vec::new())), StartView::Files);
        assert_eq!(super::start_view(&super::Start::Files(vec!["a.md".into()])), StartView::Files);
    }
}
