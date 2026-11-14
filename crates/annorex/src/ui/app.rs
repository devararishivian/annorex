//! The state of the viewer and its keys.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use super::cards::MAX_PICK;
use super::editor::{Editor, EditorResult};
use super::focus::Mode;
use super::hits::HitMap;
use super::mouse::{Scroll, Tops};
use super::tree::{Finder, Tree};
use crate::agent::Agent;
use crate::config::Config;
use crate::deliver::DeliveryMode;
use crate::doc::{Doc, DocKind};
use crate::draft::{Draft, DraftKey, Editing};
use crate::files::{LoadedFile, Scan, Stamp};
use crate::keys::Action;
use crate::payload::sanitize;
use crate::replies::{Candidate, Reply};
use crate::review::{Annotation, Kind, Review, Section, reanchor_quote, reanchor_range};
use crate::safety::Safety;
use crate::watch::{self, Change, ChangeKind, Known};
use crate::wrap::WrapCache;

/// The agent in the target pane and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentState {
    /// `None` when the pane does not run a supported agent.
    pub agent: Option<Agent>,
    pub safety: Safety,
}

impl AgentState {
    /// For example `Claude Code (idle)`, `Pi (working…)`, `OpenCode (waiting for permission)`.
    pub fn header_text(&self) -> String {
        self.text_with(|agent| agent.host().display_name())
    }

    /// The same as `header_text` with the short label, for a header too narrow for the full name.
    pub fn header_text_short(&self) -> String {
        self.text_with(|agent| agent.host().label())
    }

    fn text_with(&self, name: impl Fn(Agent) -> &'static str) -> String {
        let Some(agent) = self.agent else { return "no agent".to_owned() };
        let name = name(agent);
        let state = match &self.safety {
            Safety::Blocked { kind: Some(kind), .. } => format!("waiting for {kind}"),
            Safety::Blocked { kind: None, .. } => "blocked".to_owned(),
            Safety::Clear { state: Some(state) } if state == "working" => "working…".to_owned(),
            Safety::Clear { state: Some(state) } => state.clone(),
            Safety::Clear { state: None } => "unknown".to_owned(),
        };
        format!("{name} ({state})")
    }
}

/// What the viewer shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Viewer,
    /// More than one session can be the session of the agent. The user chooses one.
    Picker(Vec<Candidate>),
    /// A message, for example "no reply yet". `r` tries again.
    Error(String),
}

/// Where the replies come from. A problem here does not block the files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplySource {
    /// The replies of the agent are loaded.
    Ready,
    /// More than one session can be the session of the agent. The user chooses one.
    Pick(Vec<Candidate>),
    /// No replies, with the reason, for example "no reply yet". `r` tries again.
    Missing(String),
}

/// What the caller must do after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
    Reload,
    PickSession(usize),
    /// The review or the editor of the current reply changed. The caller saves the draft.
    SaveDraft,
    /// Give the notes of the reply on the screen to the agent.
    Send(DeliveryMode),
    /// The same, after the user wrote the general note (`App::export` has it).
    SendWithNote,
    /// Copy the notes to the clipboard.
    Copy,
    /// The review of the reply on the screen is gone. The caller removes the draft file.
    Discard,
    /// A new file is on the screen. The caller restores its saved draft.
    FileOpened,
}

/// The most files that the viewer keeps open.
pub(crate) const MAX_OPEN_FILES: usize = 50;

/// The id of the document of a file.
pub fn file_id(relative: &Path) -> String {
    format!("file:{}", relative.display())
}

/// What the viewer knows about an open file.
#[derive(Debug, Clone)]
pub(crate) struct OpenFile {
    pub(crate) relative: PathBuf,
    pub(crate) kind: DocKind,
    #[allow(dead_code, reason = "the watcher in a later task reads it")]
    pub(crate) stamp: Stamp,
}

/// A question that waits for a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confirm {
    /// Delete this annotation? `y` or `n`.
    Annotation(u64),
    /// Discard the notes of this reply? `y` or `n`.
    Discard,
    /// Quit with unsent notes: `s`, `k`, `d`, or `esc`.
    Quit,
    /// Close the file on the screen, which has unsent notes? `y` or `n`.
    CloseFile,
}

/// Where the keys go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    /// The file tree, on the left or over the document.
    Tree,
    Doc,
    /// The card rail, or the annotation list where the window has no rail.
    Rail,
    /// The centered box: the reply list or the finder.
    Box,
}

/// What the centered box shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoxView {
    /// The documents, or the sessions to choose from.
    Replies,
    Finder,
}

/// The parts that the last frame showed beside the document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "independent facts of one frame")]
pub(crate) struct Docked {
    /// The tree on the left.
    pub(crate) tree: bool,
    /// The card rail or the annotation list on the right.
    pub(crate) rail: bool,
    /// The card rail on the right (from 120 columns).
    pub(crate) cards: bool,
}

/// The view that the viewer starts as. It decides whether the tree shows at the start and which keys the
/// footer shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StartView {
    /// `open last`: a reply. The tree shows only after the tree key.
    Last,
    /// `open files`: the files. The tree shows on the left.
    #[default]
    Files,
}

/// The text that the user marked with `v` or `V`. The anchor is where the marking started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selection {
    Chars { anchor: usize },
    Lines { anchor: usize },
}

/// The footer message when a note key finds no text, in block mode and in text mode.
const NOTHING_TO_ANNOTATE: &str = "nothing to annotate";

/// The footer message when a note key finds no focused block in block mode.
const CHOOSE_A_BLOCK: &str = "choose a block first (j/k or a click)";

/// The footer message when the text under a note is gone after a new read.
const MOVED_TEXT: &str = "The text under your note is gone after the new read, so the editor closed.";

/// The state of the viewer.
#[derive(Debug)]
#[allow(clippy::struct_excessive_bools, reason = "independent flags of the viewer state")]
pub struct App {
    pub state: AgentState,
    pub screen: Screen,
    pub(crate) replies: Vec<Reply>,
    pub(crate) selected: usize,
    pub(crate) doc: Doc,
    pub(crate) cache: WrapCache,
    /// The `DocLine` at the top of the view. A resize keeps it.
    pub(crate) top: usize,
    /// A byte offset in the source of the document. In block mode it is the start of the focused block.
    pub(crate) cursor: usize,
    /// Block mode (the default) or text mode.
    pub(crate) mode: Mode,
    /// The width of the text in the last frame. 0 before the first frame.
    pub(crate) width: u16,
    /// The index of the first visual row on the screen in the last frame.
    pub(crate) top_row: usize,
    /// False after the user hid the tree, and at the start of the last-reply view.
    pub(crate) tree_open: bool,
    /// The view that the viewer started as.
    start_view: StartView,
    /// The annotation list is open where the window has no card rail.
    pub(crate) list_open: bool,
    /// The centered box, when one is open.
    pub(crate) boxed: Option<BoxView>,
    /// The reply list at the start of the last-reply view is open. `esc` quits there.
    pub(crate) start_box: bool,
    /// What the last frame showed beside the document. The keys `tab`, `T`, and `a` use it.
    pub(crate) docked: Docked,
    /// The cursor of the reply list.
    pub(crate) panel_index: usize,
    pub(crate) picker_index: usize,
    pub(crate) reply_source: ReplySource,
    pub(crate) focus: Focus,
    /// The text of the find input while the user types it.
    pub(crate) find_input: Option<String>,
    /// The help screen is open. Any key closes it.
    pub help_open: bool,
    pub(crate) query: Option<String>,
    /// A message for the footer.
    pub(crate) status: Option<String>,
    /// The height of the document area in the last frame.
    pub(crate) page: usize,
    /// The clickable areas of the last frame.
    pub(crate) hits: HitMap,
    /// The time and the cell of the last click in the document text, for the double click.
    pub(crate) last_click: Option<(Instant, u16, u16)>,
    /// The anchor of a drag that started in the document text.
    pub(crate) drag: Option<usize>,
    /// The scroll of the mouse wheel. Any key ends it.
    pub(crate) scroll: Scroll,
    /// The first row of each list in the last frame.
    pub(crate) tops: Tops,
    /// The text area of the document in the last frame.
    pub(crate) text_area: Rect,
    /// The time zone of the times in the reply list. The default is the system zone.
    pub(crate) zone: jiff::tz::TimeZone,
    pub(crate) config: Config,
    /// The annotations of each reply, by reply id.
    pub(crate) reviews: HashMap<String, Review>,
    /// The time of the last send of each reply that has one. A change removes the entry.
    pub(crate) sent: HashMap<String, String>,
    pub(crate) selection: Option<Selection>,
    /// The action strip shows: a selection is finished. A drag that ends, a double click, `enter` in
    /// block mode, and the `v` or `V` that ends a selection open it. Each other key in the
    /// document, a press of the mouse, and another document close it.
    pub(crate) bar_open: bool,
    pub(crate) editor: Option<Editor>,
    /// True when the terminal reports `shift+enter` as its own key. The tui sets it at start. The
    /// footer of the note box then names `shift+enter` and not `ctrl+j`.
    pub shift_enter: bool,
    /// The annotation that the editor changes. `None` for a new annotation.
    pub(crate) editing_id: Option<u64>,
    /// Editors of replies that are not on the screen.
    parked: HashMap<String, Editing>,
    pub(crate) annotation_index: usize,
    /// The annotation that waits for `y` or `n`.
    pub(crate) confirm: Option<Confirm>,
    /// The numbered pick: the ids of the notes at the focus, when the edit or remove key found more
    /// than one. The keys `1` to `9` choose one, `esc` closes it.
    pub(crate) pick: Option<Vec<u64>>,
    /// True when the pick removes the chosen note, false when it edits it.
    pub(crate) pick_removes: bool,
    /// A send or a discard from the quit question. The viewer ends when it works.
    quit_after: bool,
    exit: bool,
    /// The id of the reply in `doc`.
    pub(super) shown_id: Option<String>,
    /// Why the last save of the draft failed.
    pub(crate) save_error: Option<String>,
    /// The work directory. `None` when the viewer has no files.
    pub(crate) root: Option<PathBuf>,
    /// The open files by document id . `replies` keeps their texts after the replies, in open order.
    pub(crate) files: HashMap<String, OpenFile>,
    /// The ids of the documents whose review the last discard removed. The caller removes the drafts.
    discarded: Vec<String>,
    pub(crate) tree: Tree,
    pub(crate) finder: Finder,
    marks: HashSet<String>,
    /// The open files that changed on disk since they were read, by document id.
    pub(crate) changes: HashMap<String, ChangeKind>,
    known: Option<Known>,
    watch_rx: Option<std::sync::mpsc::Receiver<Change>>,
    /// The lists of files that the agent changed, from the thread of `changes::spawn_poll`.
    changes_rx: Option<std::sync::mpsc::Receiver<Vec<PathBuf>>>,
    /// Keeps the thread of `changes_rx` alive.
    changes_token: Option<std::sync::Arc<()>>,
    /// True after the first list: later lists can announce new files.
    changes_seen: bool,
    /// The files that are new in the changed list since the user looked at the tree.
    new_changed: usize,
    scan_rx: Option<std::sync::mpsc::Receiver<Scan>>,
    /// The ids of the newest reply, from the thread of `news::reply_looker`.
    replies_rx: Option<std::sync::mpsc::Receiver<String>>,
    /// Keeps the thread of `replies_rx` alive.
    replies_token: Option<std::sync::Arc<()>>,
    /// True when the agent has a reply that the viewer has not read.
    new_reply: bool,
    /// The notice of a newer release, from the thread of the update check.
    update_rx: Option<std::sync::mpsc::Receiver<String>>,
    /// The notice of a newer release. It shows last in the banner.
    update_notice: Option<String>,
}

impl App {
    pub fn new(state: AgentState, screen: Screen, replies: Vec<Reply>, config: &Config) -> Self {
        let mut app = Self {
            state,
            screen,
            replies: Vec::new(),
            selected: 0,
            doc: Doc::parse(""),
            cache: WrapCache::default(),
            top: 0,
            cursor: 0,
            mode: Mode::Browse,
            width: 0,
            top_row: 0,
            tree_open: true,
            start_view: StartView::default(),
            list_open: false,
            boxed: None,
            start_box: false,
            docked: Docked::default(),
            panel_index: 0,
            picker_index: 0,
            reply_source: ReplySource::Ready,
            focus: Focus::Doc,
            find_input: None,
            help_open: false,
            query: None,
            status: None,
            page: 20,
            hits: HitMap::default(),
            last_click: None,
            drag: None,
            scroll: Scroll::default(),
            tops: Tops::default(),
            text_area: Rect::default(),
            zone: jiff::tz::TimeZone::system(),
            config: config.clone(),
            reviews: HashMap::new(),
            sent: HashMap::new(),
            selection: None,
            bar_open: false,
            editor: None,
            shift_enter: false,
            editing_id: None,
            parked: HashMap::new(),
            annotation_index: 0,
            confirm: None,
            pick: None,
            pick_removes: false,
            quit_after: false,
            exit: false,
            shown_id: None,
            save_error: None,
            root: None,
            files: HashMap::new(),
            discarded: Vec::new(),
            tree: Tree::default(),
            finder: Finder::default(),
            marks: HashSet::new(),
            changes: HashMap::new(),
            known: None,
            watch_rx: None,
            changes_rx: None,
            changes_token: None,
            changes_seen: false,
            new_changed: 0,
            scan_rx: None,
            replies_rx: None,
            replies_token: None,
            new_reply: false,
            update_rx: None,
            update_notice: None,
        };
        if !replies.is_empty() {
            app.replies = replies;
            app.show_reply(0);
        }
        app
    }

    /// Add the files of the work directory `root` to the viewer. Open files, the tree, and the finder need it.
    #[must_use]
    pub fn with_root(mut self, root: PathBuf) -> Self {
        self.root = Some(root);
        self
    }

    /// Start the viewer as `view`. The last-reply view starts with the tree hidden.
    #[must_use]
    pub fn with_start_view(mut self, view: StartView) -> Self {
        self.start_view = view;
        self.tree_open = view == StartView::Files;
        self
    }

    /// Open the reply list at the start of the last-reply view, with the newest reply selected. One reply
    /// or none gives no list. `esc` quits from this list.
    pub fn open_start_list(&mut self) {
        let ready = self.screen == Screen::Viewer && self.reply_source == ReplySource::Ready;
        if self.start_view == StartView::Last && ready && self.replies.len() >= 2 && self.boxed.is_none() {
            self.boxed = Some(BoxView::Replies);
            self.focus = Focus::Box;
            self.panel_index = 0;
            self.start_box = true;
        }
    }

    /// The view that the viewer started as.
    pub(crate) fn start_view(&self) -> StartView {
        self.start_view
    }

    /// The id of the document on the screen. `None` when there is none.
    pub(crate) fn shown_id(&self) -> Option<&str> {
        self.shown_id.as_deref()
    }

    /// True when the viewer has a work directory, so it has files.
    pub fn has_root(&self) -> bool {
        self.root.is_some()
    }

    /// True when `id` is the id of an open file.
    pub fn is_file(&self, id: &str) -> bool {
        self.files.contains_key(id)
    }

    /// Add a file to the open files and show it. An open file is only selected.
    pub fn open_file(&mut self, loaded: LoadedFile) -> Effect {
        let id = file_id(&loaded.relative);
        if let Some(index) = self.replies.iter().position(|reply| reply.id == id) {
            self.show_reply(index);
            return Effect::None;
        }
        if self.files.len() >= MAX_OPEN_FILES {
            self.status = Some("Too many open files. Close one with W.".to_owned());
            return Effect::None;
        }
        self.files.insert(
            id.clone(),
            OpenFile { kind: DocKind::of(&loaded.relative), relative: loaded.relative, stamp: loaded.stamp },
        );
        self.replies.push(Reply { id, text: loaded.text, at: None });
        self.screen = Screen::Viewer;
        // A file on the screen is the work now: the choice of a session waits in the reply list.
        if matches!(self.reply_source, ReplySource::Pick(_)) && self.boxed == Some(BoxView::Replies) {
            self.close_box();
        }
        self.show_reply(self.replies.len() - 1);
        self.sync_watch();
        Effect::None
    }

    /// The path of the file on the screen, relative to the root.
    pub fn shown_file(&self) -> Option<&Path> {
        self.files.get(self.shown_id.as_deref()?).map(|file| file.relative.as_path())
    }

    /// Put the result of a scan of the work directory in the tree.
    pub fn set_scan(&mut self, scan: Scan) {
        self.tree.load(scan);
        // A finder that opened before the scan ended has no paths yet.
        self.finder.set_paths(self.tree.file_paths());
    }

    /// Set the files that the agent changed in its last turns. The tree and the finder put them first.
    pub fn set_changed(&mut self, changed: Vec<PathBuf>) {
        self.tree.set_changed(changed);
        self.finder.set_paths(self.tree.file_paths());
    }

    /// Scan the work directory on a thread. [`App::poll_scan`] takes the result.
    pub fn start_scan(&mut self) {
        let Some(root) = self.root.clone() else { return };
        let (sender, receiver) = std::sync::mpsc::channel();
        self.scan_rx = Some(receiver);
        std::thread::spawn(move || {
            // The viewer can end before the scan: then nobody reads the result.
            let _ = sender.send(crate::files::scan(&root));
        });
    }

    /// Take the result of the scan when it is ready. True when the tree changed.
    pub fn poll_scan(&mut self) -> bool {
        let Some(receiver) = &self.scan_rx else { return false };
        match receiver.try_recv() {
            Ok(scan) => {
                self.scan_rx = None;
                self.set_scan(scan);
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.scan_rx = None;
                self.set_scan(Scan { entries: Vec::new(), truncated: false });
                true
            }
        }
    }

    /// The ids of the documents with a saved draft that no send covered. Their files show `●`.
    pub fn set_draft_marks(&mut self, ids: &[String]) {
        self.marks = ids.iter().cloned().collect();
    }

    /// True when the file has notes that no send took: in the open file, or in a draft on disk.
    pub(crate) fn is_marked(&self, relative: &Path) -> bool {
        let id = file_id(relative);
        match self.reviews.get(&id) {
            Some(review) => review.has_unsent(),
            None => self.marks.contains(&id),
        }
    }

    /// Read a file of the work directory and show it. A failure goes to the footer.
    pub(super) fn open_path(&mut self, relative: &Path) -> Effect {
        let Some(root) = self.root.clone() else { return Effect::None };
        match crate::files::load(&root, relative) {
            Ok(loaded) => {
                let before = self.replies.len();
                self.open_file(loaded);
                if self.replies.len() > before { Effect::FileOpened } else { Effect::None }
            }
            Err(error) => {
                self.status = Some(error.to_string());
                Effect::None
            }
        }
    }

    /// Show the file tree. The viewer starts with it when no file is open.
    pub fn open_tree(&mut self) {
        self.tree_open = true;
        self.close_box_only();
        self.focus_tree();
    }

    /// Give the tree the focus. The user sees the changed files now, so the banner about them ends.
    fn focus_tree(&mut self) {
        self.focus = Focus::Tree;
        self.tree.stop_filter();
        self.new_changed = 0;
    }

    /// The tree is on the screen at the left of the document. The user sees the changed files, so the
    /// banner about them ends.
    pub(crate) fn tree_seen(&mut self) {
        self.new_changed = 0;
    }

    /// Show the tree with a directory open and the cursor on it.
    pub fn reveal_dir(&mut self, relative: &Path) {
        self.tree.reveal(relative);
        self.open_tree();
    }

    /// Tell the watch thread which files are open and what they looked like when read.
    fn sync_watch(&self) {
        let Some(known) = &self.known else { return };
        let list: Vec<(PathBuf, Stamp)> = self
            .replies
            .iter()
            .filter_map(|reply| self.files.get(&reply.id))
            .map(|file| (file.relative.clone(), file.stamp))
            .collect();
        if let Ok(mut guard) = known.lock() {
            *guard = list;
        }
    }

    /// Start the thread that looks at the open files every two seconds.
    pub fn start_watch(&mut self) {
        let Some(root) = self.root.clone() else { return };
        let known: Known = Known::default();
        self.watch_rx = Some(watch::spawn(root, &known));
        self.known = Some(known);
        self.sync_watch();
    }

    /// Start the thread that looks at the transcript of the agent. `look` is the look of
    /// [`crate::changes::looker`].
    pub fn start_changes(&mut self, look: impl FnMut() -> Option<Vec<PathBuf>> + Send + 'static) {
        let (receiver, token) = crate::changes::spawn_poll(crate::changes::EVERY, look);
        self.changes_rx = Some(receiver);
        self.changes_token = Some(token);
    }

    /// Watch for new replies on a thread. A new look replaces the old thread.
    pub fn start_reply_poll(&mut self, look: impl FnMut() -> Option<String> + Send + 'static) {
        let (receiver, token) = crate::changes::spawn_poll(crate::changes::EVERY, look);
        self.replies_rx = Some(receiver);
        self.replies_token = Some(token);
    }

    /// Take the notice of a newer release from the thread of the update check.
    pub fn start_update_check(&mut self, rx: std::sync::mpsc::Receiver<String>) {
        self.update_rx = Some(rx);
    }

    /// Keep the update check of `old` when this app replaces it.
    pub(crate) fn keep_update_check(&mut self, old: &mut App) {
        self.update_rx = old.update_rx.take();
        self.update_notice = old.update_notice.take();
    }

    /// Stop watching for new replies.
    pub fn stop_reply_poll(&mut self) {
        self.replies_rx = None;
        self.replies_token = None;
    }

    /// The newest reply of the agent has the id `newest`. The banner shows when the viewer has another one.
    pub fn reply_polled(&mut self, newest: &str) {
        let mine =
            self.replies.iter().find(|reply| !self.files.contains_key(&reply.id)).map(|r| r.id.as_str());
        self.new_reply = mine != Some(newest);
    }

    /// Take a new list of changed files. A later list that holds files the tree did not have
    /// raises the banner.
    pub fn changed_list(&mut self, list: Vec<PathBuf>) {
        let fresh = list.iter().filter(|path| !self.tree.is_changed(path)).count();
        if self.changes_seen && self.focus != Focus::Tree && !self.docked.tree {
            self.new_changed += fresh;
        }
        self.changes_seen = true;
        // A file that the agent created after the scan is not in the tree: scan again.
        if self.tree.is_loaded()
            && self.scan_rx.is_none()
            && list.iter().any(|path| !self.tree.has_file(path))
        {
            self.start_scan();
        }
        self.set_changed(list);
    }

    /// Take what the background threads found: the scan of the tree and the changes of files.
    /// True when something changed on the screen.
    pub fn poll_background(&mut self) -> bool {
        let mut changed = self.poll_scan();
        while let Some(list) = self.changes_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.changed_list(list);
            changed = true;
        }
        while let Some(id) = self.replies_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.reply_polled(&id);
            changed = true;
        }
        while let Some(change) = self.watch_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.file_changed(&change);
            changed = true;
        }
        while let Some(notice) = self.update_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.update_notice = Some(notice);
            changed = true;
        }
        changed
    }

    /// A file changed on disk. The text on the screen stays until the user reads the file again.
    pub fn file_changed(&mut self, change: &Change) {
        let id = file_id(&change.path);
        let Some(file) = self.files.get(&id) else { return };
        // The message can be old: the user can read the file again before it arrives. Look at the file now.
        let current = self.root.as_deref().map(|root| crate::files::stamp(root, &change.path));
        match current {
            Some(Some(now)) if now == file.stamp => {}
            Some(None) => {
                self.changes.insert(id, ChangeKind::Deleted);
            }
            Some(Some(_)) => {
                self.changes.insert(id, ChangeKind::Modified);
            }
            None => {
                self.changes.insert(id, change.kind);
            }
        }
    }

    /// The banner: the file on the screen changed on disk, the agent has news, and a newer release exists.
    pub(crate) fn banner(&self) -> Option<String> {
        let parts: Vec<String> = [self.file_banner(), self.agent_banner(), self.update_notice.clone()]
            .into_iter()
            .flatten()
            .collect();
        (!parts.is_empty()).then(|| parts.join("  |  "))
    }

    /// The banner for new files that the agent changed, until the user opens the tree.
    fn agent_banner(&self) -> Option<String> {
        let n = self.new_changed;
        let files = (n > 0).then(|| {
            let files = if n == 1 { "file" } else { "files" };
            format!("The agent changed {n} more {files}. They are at the top of the tree (T).")
        });
        let reply = self.new_reply.then(|| "The agent has a new reply. Press r to read it.".to_owned());
        match (reply, files) {
            (Some(reply), Some(files)) => Some(format!("{reply}  |  {files}")),
            (reply, files) => reply.or(files),
        }
    }

    fn file_banner(&self) -> Option<String> {
        let id = self.shown_id.as_deref()?;
        let path = self.files.get(id)?.relative.display();
        Some(match self.changes.get(id)? {
            ChangeKind::Modified => format!("{path} changed on disk. Press r to read it again."),
            ChangeKind::Deleted => format!("{path} was deleted. Your notes are kept."),
        })
    }

    /// `r` in the file viewer: read the file on the screen again.
    fn reload_shown(&mut self) {
        let (Some(root), Some(id)) = (self.root.clone(), self.shown_id.clone()) else { return };
        let Some(relative) = self.files.get(&id).map(|file| file.relative.clone()) else { return };
        match crate::files::load(&root, &relative) {
            Ok(loaded) => self.reload_file(&id, loaded),
            Err(crate::files::FileError::NotFound(_) | crate::files::FileError::NotAFile(_)) => {
                self.changes.insert(id, ChangeKind::Deleted);
                self.status = Some("The file is gone. Your notes are kept.".to_owned());
            }
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    /// Give an open file its new text. The notes and the open editors move to their quoted text. A
    /// note whose text is gone stays and is marked. The cursor stays on its line.
    pub fn reload_file(&mut self, id: &str, loaded: LoadedFile) {
        let Some(position) = self.replies.iter().position(|reply| reply.id == id) else { return };
        let Some(reply) = self.replies.get_mut(position) else { return };
        let old = sanitize(&reply.text);
        let new = sanitize(&loaded.text);
        reply.text = loaded.text;
        if let Some(file) = self.files.get_mut(id) {
            file.stamp = loaded.stamp;
        }
        self.changes.remove(id);
        let changed = if old == new { Vec::new() } else { vec![(id.to_owned(), old, new)] };
        let selected = self.selection_before();
        if self.shown_id.as_deref() == Some(id) {
            let (line, top, focus) = (self.cursor_line(), self.top, self.focus_range());
            self.show_reply(position);
            self.top = top.min(self.doc.lines.len().saturating_sub(1));
            let line = line.min(self.doc.lines.len().saturating_sub(1));
            self.cursor = self.line_start(line).unwrap_or(0);
            self.snap_cursor();
            self.follow_focus(focus, &changed);
        }
        self.place_again(&changed);
        self.follow_selection(selected, &changed);
        self.sync_watch();
    }

    /// `[` and `]`: show the previous or the next open file.
    fn step_file(&mut self, forward: bool) {
        let target = if forward { Some(self.selected + 1) } else { self.selected.checked_sub(1) };
        if let Some(index) = target.filter(|&index| index < self.replies.len()) {
            self.show_reply(index);
        }
    }

    /// `W`: close the file on the screen. A file with unsent notes asks first.
    fn begin_close(&mut self) {
        if self.shown_file().is_none() {
            return;
        }
        match self.review() {
            Some(review) if review.has_unsent() && self.save_error.is_some() => {
                self.status =
                    Some("Not closed: the draft cannot be saved, so the notes would be lost.".to_owned());
            }
            Some(review) if review.has_unsent() => self.confirm = Some(Confirm::CloseFile),
            _ => self.close_shown(),
        }
    }

    /// Remove the file on the screen from the open files. Its notes stay in its draft on disk.
    fn close_shown(&mut self) {
        let Some(id) = self.shown_id.take() else { return };
        let index = self.selected.min(self.replies.len().saturating_sub(1));
        if index < self.replies.len() {
            self.replies.remove(index);
        }
        self.files.remove(&id);
        self.changes.remove(&id);
        self.reviews.remove(&id);
        self.sent.remove(&id);
        self.parked.remove(&id);
        self.editor = None;
        self.editing_id = None;
        self.selection = None;
        self.confirm = None;
        self.annotation_index = 0;
        if self.replies.is_empty() {
            self.doc = Doc::parse("");
            self.cache.clear();
            self.selected = 0;
            self.panel_index = 0;
            self.cursor = 0;
            self.top = 0;
            self.mode = Mode::Browse;
        } else {
            self.show_reply(index.min(self.replies.len() - 1));
        }
        self.sync_watch();
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    /// The `DocLine` of the cursor.
    pub fn cursor_line(&self) -> usize {
        self.doc.locate(self.cursor).0
    }

    /// The cursor, as a byte offset in the source of the document.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The first cursor position of a `DocLine`.
    pub(super) fn line_start(&self, index: usize) -> Option<usize> {
        let line = self.doc.lines.get(index)?;
        Some(self.doc.stops(index).first().copied().unwrap_or(line.source.start))
    }

    /// Put `doc_line` at the top of the view and the cursor on it, in text mode.
    pub fn jump_to_line(&mut self, doc_line: usize) {
        if let Some(start) = self.line_start(doc_line) {
            self.top = doc_line;
            self.cursor = start;
            self.mode = Mode::Text;
        }
    }

    /// Move the cursor to the nearest stop at or before it, so it is always a stop.
    fn snap_cursor(&mut self) {
        let line = self.doc.locate(self.cursor).0;
        let stops = self.doc.stops(line);
        let cursor = self.cursor;
        let snapped = stops.iter().rev().find(|&&stop| stop <= cursor).or(stops.first()).copied();
        self.cursor = snapped.or_else(|| self.line_start(line)).unwrap_or(0);
    }

    /// New replies after a read. The selected reply stays when it is still there.
    pub fn set_replies(&mut self, replies: Vec<Reply>) {
        // Keep an older reply that the user chose. On the newest reply, go to a new newest reply, but
        // keep the position when no new reply came.
        let current = self.replies.get(self.selected).map(|reply| reply.id.clone());
        // An open reply list keeps its highlight on the same reply.
        let highlighted = (self.focus == Focus::Box && self.boxed == Some(BoxView::Replies))
            .then(|| self.replies.get(self.panel_index).map(|reply| reply.id.clone()))
            .flatten();
        let (top, cursor, focus) = (self.top, self.cursor, self.focus_range());
        let selected = self.selection_before();
        // The open files stay after the replies.
        let mut replies = replies;
        replies.extend(self.replies.iter().filter(|reply| self.files.contains_key(&reply.id)).cloned());
        let on_file = current.as_ref().is_some_and(|id| self.files.contains_key(id));
        let keep = current
            .and_then(|id| replies.iter().position(|reply| reply.id == id))
            .filter(|&index| on_file || self.selected > 0 || index == 0);
        let changed = self.changed_texts(&replies);
        self.replies = replies;
        self.screen = Screen::Viewer;
        self.reply_source = ReplySource::Ready;
        self.new_reply = false;
        match keep {
            Some(index) => {
                self.show_reply(index);
                self.top = top.min(self.doc.lines.len().saturating_sub(1));
                self.cursor = cursor.min(self.doc.source.len());
                self.snap_cursor();
                self.follow_focus(focus, &changed);
            }
            None => self.show_reply(0),
        }
        if let Some(index) = highlighted.and_then(|id| self.replies.iter().position(|reply| reply.id == id)) {
            self.panel_index = index;
        }
        self.place_again(&changed);
        self.follow_selection(selected, &changed);
    }

    /// Move the selection of the shown reply to its text in the new source. `old` is the range of the
    /// selection before the change. A selection whose text is gone ends, and the footer says so.
    fn follow_selection(
        &mut self,
        old: Option<(String, Range<usize>)>,
        changed: &[(String, String, String)],
    ) {
        let Some((id, range)) = old else { return };
        // The shown reply must be the same one, with its selection still on.
        if self.shown_id.as_deref() != Some(id.as_str()) || self.selection.is_none() {
            return;
        }
        let id = id.as_str();
        let Some((_, old_source, new_source)) = changed.iter().find(|(changed_id, ..)| changed_id == id)
        else {
            return;
        };
        let Some(moved) = reanchor_range(old_source, new_source, &range) else {
            self.selection = None;
            self.bar_open = false;
            self.status = Some(MOVED_TEXT.to_owned());
            return;
        };
        let last = self.doc.source.get(..moved.end).and_then(|head| head.char_indices().next_back());
        let end = last.map_or(moved.start, |(index, _)| index.max(moved.start));
        self.selection = match self.selection {
            Some(Selection::Chars { .. }) => {
                Some(Selection::Chars { anchor: self.stop_at_or_before(moved.start) })
            }
            Some(Selection::Lines { .. }) => Some(Selection::Lines { anchor: moved.start }),
            None => None,
        };
        self.cursor = end;
        self.snap_cursor();
    }

    /// The id of the shown reply and the range of its selection, before a change of the text.
    fn selection_before(&self) -> Option<(String, Range<usize>)> {
        Some((self.shown_id.clone()?, self.selection_range()?))
    }

    /// The nearest cursor stop at or before `offset`.
    fn stop_at_or_before(&self, offset: usize) -> usize {
        let line = self.doc.locate(offset).0;
        let stops = self.doc.stops(line);
        stops.iter().rev().find(|&&stop| stop <= offset).or(stops.first()).copied().unwrap_or(offset)
    }

    /// The replies that stay but have a new text: id, old source, new source.
    fn changed_texts(&self, replies: &[Reply]) -> Vec<(String, String, String)> {
        replies
            .iter()
            .filter_map(|new| {
                let old = self.replies.iter().find(|old| old.id == new.id)?;
                (old.text != new.text).then(|| (new.id.clone(), sanitize(&old.text), sanitize(&new.text)))
            })
            .collect()
    }

    /// Put the annotations, the open editor, and the parked editors of changed replies on their
    /// quoted text again. An editor whose text is gone closes, and the footer says so.
    fn place_again(&mut self, changed: &[(String, String, String)]) {
        if !changed.is_empty() {
            self.pick = None;
        }
        for (id, old, new) in changed {
            if let Some(review) = self.reviews.get_mut(id) {
                review.reanchor(new);
            }
            let lost = self.parked.get_mut(id).filter(|editing| !editing.general).is_some_and(|editing| {
                match reanchor_range(old, new, &editing.range) {
                    Some(range) => {
                        editing.range = range;
                        false
                    }
                    None => true,
                }
            });
            if lost {
                self.parked.remove(id);
                self.status = Some(MOVED_TEXT.to_owned());
            }
            if self.shown_id.as_deref() == Some(id.as_str()) {
                let moved = self
                    .editor
                    .as_ref()
                    .filter(|editor| !editor.general)
                    .map(|editor| reanchor_range(old, new, &editor.range));
                match moved {
                    Some(Some(range)) => {
                        if let Some(editor) = self.editor.as_mut() {
                            editor.range = range;
                        }
                    }
                    Some(None) => {
                        self.editor = None;
                        self.editing_id = None;
                        self.status = Some(MOVED_TEXT.to_owned());
                    }
                    None => {}
                }
            }
        }
    }

    pub fn set_state(&mut self, state: AgentState) {
        self.state = state;
    }

    /// Set the time zone of the times in the reply list.
    pub fn set_time_zone(&mut self, zone: jiff::tz::TimeZone) {
        self.zone = zone;
    }

    /// Show the session picker, with the first session selected.
    pub fn set_picker(&mut self, candidates: Vec<Candidate>) {
        if self.root.is_some() {
            self.set_reply_source(ReplySource::Pick(candidates));
            return;
        }
        self.help_open = false;
        self.screen = Screen::Picker(candidates);
        self.picker_index = 0;
    }

    /// A message instead of the viewer. With files, the problem is about the replies only: the reply list
    /// shows it and the files stay usable.
    pub fn set_error(&mut self, message: String) {
        if self.root.is_some() {
            self.set_reply_source(ReplySource::Missing(message));
            return;
        }
        // The help belongs to the viewer. The next key must act on the message.
        self.help_open = false;
        self.screen = Screen::Error(message);
    }

    /// Tell where the replies come from. A choice of sessions opens the reply list when no document is
    /// on the screen.
    pub fn set_reply_source(&mut self, source: ReplySource) {
        self.picker_index = 0;
        if matches!(source, ReplySource::Pick(_)) && self.shown_id.is_none() {
            self.boxed = Some(BoxView::Replies);
            self.focus = Focus::Box;
        }
        self.reply_source = source;
        self.screen = Screen::Viewer;
    }

    pub fn reply_source(&self) -> &ReplySource {
        &self.reply_source
    }

    /// Show a message in the footer until the next key.
    pub fn set_status(&mut self, message: String) {
        self.status = Some(message);
    }

    /// The selected reply (0 is the newest) and the number of replies.
    pub fn reply_position(&self) -> (usize, usize) {
        (self.selected, self.replies.len())
    }

    fn show_reply(&mut self, index: usize) {
        let Some(reply) = self.replies.get(index) else { return };
        let id = reply.id.clone();
        self.selected = index;
        self.panel_index = index;
        let outgoing = self.editor.as_ref().map(|editor| self.editing_of(editor));
        // The text can be new, so the ids of the numbered pick can be stale.
        self.pick = None;
        self.doc = match self.files.get(&id) {
            Some(file) => Doc::for_kind(file.kind, &reply.text),
            None => Doc::parse(&reply.text),
        };
        self.cache.clear();
        self.top = 0;
        // The wheel scrolled the old text: the view of the new text follows its focus again.
        self.scroll.doc = false;
        // A drag or a double click of the old text must not carry over to the new text.
        self.drag = None;
        self.last_click = None;
        let first = self.doc.lines.iter().position(|line| !line.is_blank());
        self.cursor = first.and_then(|index| self.line_start(index)).unwrap_or(0);
        if self.shown_id.as_deref() != Some(id.as_str()) {
            // Another reply: its editor was parked, and a selection or a question do not carry over.
            if let (Some(editing), Some(old)) = (outgoing, self.shown_id.take()) {
                self.editor = None;
                self.parked.insert(old, editing);
            }
            self.editing_id = None;
            self.selection = None;
            self.bar_open = false;
            self.mode = Mode::Browse;
            self.cursor = self.block_start(0).unwrap_or(self.cursor);
            self.confirm = None;
            self.annotation_index = 0;
            self.shown_id = Some(id.clone());
            if let Some(editing) = self.parked.remove(&id) {
                self.editing_id = editing.annotation;
                self.editor = Some(Editor::from_editing(editing));
            }
        }
    }

    /// The review of the reply on the screen.
    pub fn review(&self) -> Option<&Review> {
        self.reviews.get(self.shown_id.as_deref()?)
    }

    /// The open editor.
    pub fn editor(&self) -> Option<&Editor> {
        self.editor.as_ref()
    }

    /// The message of the footer.
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// True when the app already has a review for the reply `id`.
    pub fn has_review(&self, id: &str) -> bool {
        self.reviews.contains_key(id)
    }

    /// The ids of all replies, newest first.
    pub fn reply_ids(&self) -> Vec<String> {
        self.replies.iter().map(|reply| reply.id.clone()).collect()
    }

    /// Show the result of the last save of the draft in the footer. `None` clears it.
    pub fn set_save_error(&mut self, error: Option<String>) {
        self.save_error = error;
    }

    /// The draft of the reply on the screen: the review, the open editor, and the send time.
    pub fn draft(&self, agent: Agent, session: &str) -> Option<Draft> {
        self.draft_of(self.shown_id.as_deref()?, agent, session)
    }

    /// The drafts to save after a send: every document. `session_of` gives the session part of the key
    /// of a document id. A document without a session has no draft.
    pub fn drafts(&self, agent: Agent, session_of: impl Fn(&str) -> Option<String>) -> Vec<Draft> {
        self.replies
            .iter()
            .filter_map(|reply| self.draft_of(&reply.id, agent, &session_of(&reply.id)?))
            .filter(|draft| !draft.review.is_empty() || draft.editing.is_some() || draft.sent_at.is_some())
            .collect()
    }

    fn draft_of(&self, id: &str, agent: Agent, session: &str) -> Option<Draft> {
        self.replies.iter().find(|reply| reply.id == id)?;
        let key = DraftKey { agent, session: session.to_owned(), document: id.to_owned() };
        let name = self.name_of(id);
        let mut draft = Draft::new(key, &name);
        draft.review = self.reviews.get(id).cloned().unwrap_or_default();
        draft.editing = if self.shown_id.as_deref() == Some(id) {
            self.editor.as_ref().map(|editor| self.editing_of(editor))
        } else {
            self.parked.get(id).cloned()
        };
        draft.sent_at = self.sent.get(id).cloned();
        Some(draft)
    }

    /// The state of an editor for a draft, with the text under its range in the shown document.
    fn editing_of(&self, editor: &Editor) -> Editing {
        let quote = (!editor.general).then(|| self.doc.source.get(editor.range.clone()).map(str::to_owned));
        Editing { annotation: self.editing_id, quote: quote.flatten(), ..editor.to_editing() }
    }

    /// Put a saved draft back: the review (checked against the reply), the send time, and the
    /// editor that was open.
    pub fn restore(&mut self, reply_id: &str, draft: Draft) {
        let Draft { mut review, mut editing, sent_at, .. } = draft;
        if let Some(reply) = self.replies.iter().find(|reply| reply.id == reply_id) {
            let source = sanitize(&reply.text);
            review.reanchor(&source);
            // The document can have changed since the save: follow the text under the editor.
            if let Some(open) = editing.as_mut().filter(|open| !open.general) {
                let moved =
                    open.quote.as_deref().and_then(|quote| reanchor_quote(quote, &source, &open.range));
                if let Some(range) = moved {
                    open.range = range;
                } else {
                    editing = None;
                    self.status = Some(MOVED_TEXT.to_owned());
                }
            }
        }
        // A draft of M2 keeps the send time only on the draft: all its notes were sent then.
        if let Some(at) = &sent_at {
            review.mark_sent(at);
        }
        self.reviews.insert(reply_id.to_owned(), review);
        match sent_at {
            Some(at) => self.sent.insert(reply_id.to_owned(), at),
            None => self.sent.remove(reply_id),
        };
        if let Some(editing) = editing {
            if self.shown_id.as_deref() == Some(reply_id) {
                self.editing_id = editing.annotation;
                self.editor = Some(Editor::from_editing(editing));
            } else {
                self.parked.insert(reply_id.to_owned(), editing);
            }
        }
    }

    /// Record a send or a paste of the reply on the screen.
    pub fn mark_sent(&mut self, at: &str) {
        for reply in &self.replies {
            if let Some(review) = self.reviews.get_mut(&reply.id) {
                review.mark_sent(at);
                self.sent.insert(reply.id.clone(), at.to_owned());
            }
        }
    }

    /// The id of the reply on the screen.
    pub fn shown_reply_id(&self) -> Option<&str> {
        self.shown_id.as_deref()
    }

    /// The newest send time of the documents, when no document has an unsent note.
    pub fn shown_sent(&self) -> Option<&str> {
        if self.unsent_file_count() > 0 {
            return None;
        }
        self.replies.iter().filter_map(|reply| self.sent.get(&reply.id)).max().map(String::as_str)
    }

    /// The numbers (1 is the newest) of the replies with annotations that no send has covered.
    pub fn unsent_replies(&self) -> Vec<usize> {
        self.replies
            .iter()
            .enumerate()
            .filter(|(_, reply)| {
                !self.files.contains_key(&reply.id)
                    && self.reviews.get(&reply.id).is_some_and(Review::has_unsent)
            })
            .map(|(index, _)| index + 1)
            .collect()
    }

    /// Handle one key.
    /// Text that the terminal pasted. It goes into the open editor as text. Without an editor,
    /// it does nothing, so a paste never works as a list of commands.
    pub fn handle_paste(&mut self, text: &str) -> Effect {
        let Some(editor) = &mut self.editor else { return Effect::None };
        self.status = None;
        editor.push_str(&sanitize(text));
        Effect::SaveDraft
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if key.kind != KeyEventKind::Press {
            return Effect::None;
        }
        // A key brings the views that the wheel scrolled back to their focus.
        self.scroll = Scroll::default();
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            // Like `q`: an open editor closes first, the first press asks about unsent notes, the
            // second press quits. The draft stays on disk.
            if self.editor.is_some() {
                self.editor = None;
                self.editing_id = None;
                return Effect::SaveDraft;
            }
            if self.confirm == Some(Confirm::Quit) {
                return Effect::Quit;
            }
            self.confirm = None;
            return self.request_quit();
        }
        self.status = None;
        if self.editor.is_some() {
            return self.editor_key(key);
        }
        if let Some(what) = self.confirm {
            return self.confirm_key(key, what);
        }
        if let Some(effect) = self.pick_key(key) {
            return effect;
        }
        if self.find_input.is_some() {
            self.find_key(key);
            return Effect::None;
        }
        if self.help_open {
            self.help_open = false;
            return Effect::None;
        }
        // The footer shows the bound keys, so the two screens accept them next to the fixed keys.
        let action = self.action_of(key);
        match &self.screen {
            Screen::Error(_) => match key.code {
                KeyCode::Char('r') => Effect::Reload,
                KeyCode::Char('q') => Effect::Quit,
                _ => match action {
                    Some(Action::Reload) => Effect::Reload,
                    Some(Action::Quit) => Effect::Quit,
                    _ => Effect::None,
                },
            },
            Screen::Picker(candidates) => {
                let count = candidates.len();
                let code = match action {
                    Some(Action::Down) => KeyCode::Char('j'),
                    Some(Action::Up) => KeyCode::Char('k'),
                    Some(Action::Quit) => KeyCode::Char('q'),
                    _ => key.code,
                };
                match code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.picker_index = (self.picker_index + 1).min(count.saturating_sub(1));
                        Effect::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.picker_index = self.picker_index.saturating_sub(1);
                        Effect::None
                    }
                    KeyCode::Enter if count > 0 => Effect::PickSession(self.picker_index),
                    KeyCode::Char('q') => Effect::Quit,
                    _ => Effect::None,
                }
            }
            Screen::Viewer => match self.focus {
                Focus::Tree => self.tree_key(key),
                Focus::Doc => self.doc_key(key),
                Focus::Rail => self.rail_key(key),
                Focus::Box => self.box_key(key),
            },
        }
    }

    /// The action of a key. The arrow keys always move, whatever the config says.
    pub(super) fn action_of(&self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Down => Some(Action::Down),
            KeyCode::Up => Some(Action::Up),
            KeyCode::Left => Some(Action::Left),
            KeyCode::Right => Some(Action::Right),
            _ => self.config.keys.action(key),
        }
    }

    #[allow(clippy::too_many_lines, reason = "one match over the actions of the document")]
    fn doc_key(&mut self, key: KeyEvent) -> Effect {
        // Each key closes the action strip: a move, `esc`, a note key, and a key that opens a part.
        // The keys that end a selection open it again. The help screen only covers it.
        let strip_was_open = std::mem::take(&mut self.bar_open);
        if self.action_of(key) == Some(Action::Help) {
            self.bar_open = strip_was_open;
        }
        // `esc` with nothing selected and nothing focused is the top level: it quits, like `q`.
        if key.code == KeyCode::Esc && self.mode == Mode::Browse && !strip_was_open {
            return self.request_quit();
        }
        if self.in_block_mode() && self.block_key(key) {
            return Effect::None;
        }
        let half = signed((self.page / 2).max(1));
        match key.code {
            KeyCode::PageDown => self.move_cursor(signed(self.page.max(1))),
            KeyCode::PageUp => self.move_cursor(-signed(self.page.max(1))),
            KeyCode::Home => self.set_cursor_line(0),
            KeyCode::End => self.set_cursor_line(self.doc.lines.len().saturating_sub(1)),
            KeyCode::Tab => self.cycle_focus(),
            KeyCode::Esc => self.escape(),
            _ => match self.action_of(key) {
                Some(Action::Quit) => return self.request_quit(),
                Some(Action::Reload) => {
                    self.reload_shown();
                    return Effect::Reload;
                }
                Some(Action::Down) => self.move_cursor(1),
                Some(Action::Up) => self.move_cursor(-1),
                Some(Action::Left) => self.step(false),
                Some(Action::Right) => self.step(true),
                Some(Action::WordNext) => self.word(true),
                Some(Action::WordPrev) => self.word(false),
                Some(Action::LineStart) => self.line_edge(false),
                Some(Action::LineEnd) => self.line_edge(true),
                Some(Action::HalfPageDown) => self.move_cursor(half),
                Some(Action::HalfPageUp) => self.move_cursor(-half),
                Some(Action::Top) => self.set_cursor_line(0),
                Some(Action::Bottom) => self.set_cursor_line(self.doc.lines.len().saturating_sub(1)),
                Some(Action::Find) => self.find_input = Some(String::new()),
                Some(Action::Help) => self.help_open = true,
                Some(Action::FindNext) => self.find_next(true, false),
                Some(Action::FindPrev) => self.find_next(false, false),
                Some(Action::Replies) => self.toggle_box(BoxView::Replies),
                Some(Action::Annotations) => self.toggle_list(),
                Some(Action::PrevFile) if self.root.is_some() => self.step_file(false),
                Some(Action::NextFile) if self.root.is_some() => self.step_file(true),
                Some(Action::CloseFile) if self.shown_file().is_some() => self.begin_close(),
                Some(Action::Tree) if self.root.is_some() => self.toggle_tree(),
                Some(Action::FindFile) if self.root.is_some() => self.toggle_box(BoxView::Finder),
                Some(Action::Select) => self.toggle_selection(false, strip_was_open),
                Some(Action::SelectLines) => self.toggle_selection(true, strip_was_open),
                Some(Action::Comment) => return self.begin_new(Kind::Comment),
                Some(Action::LooksGood) => return self.add_looks_good(),
                Some(Action::Delete) => return self.add_instant(Kind::Delete),
                Some(Action::Replace) => return self.begin_new(Kind::Replace),
                Some(Action::Label) => return self.begin_label(),
                Some(Action::Edit) => return self.note_here(false),
                Some(Action::Remove) => return self.note_here(true),
                Some(Action::Send) => return self.request_send(DeliveryMode::Send),
                Some(Action::Paste) => return self.request_send(DeliveryMode::PasteOnly),
                Some(Action::SendWithNote) => return self.begin_note(),
                Some(Action::Copy) => return Effect::Copy,
                Some(Action::Discard)
                    if self.review().is_some_and(|review| !review.is_empty())
                        || self.save_error.is_some() =>
                {
                    self.confirm = Some(Confirm::Discard);
                }
                _ => {}
            },
        }
        Effect::None
    }

    /// The parts that `tab` moves through, in order: the tree, the document, and the rail. A part
    /// counts when the last frame showed it, or when it has the focus.
    fn focus_parts(&self) -> Vec<Focus> {
        let mut parts = Vec::new();
        if self.root.is_some() && (self.docked.tree || self.focus == Focus::Tree) {
            parts.push(Focus::Tree);
        }
        parts.push(Focus::Doc);
        if self.docked.rail || self.focus == Focus::Rail {
            parts.push(Focus::Rail);
        }
        parts
    }

    /// `tab`: give the focus to the next part on the screen.
    fn cycle_focus(&mut self) {
        let parts = self.focus_parts();
        let at = parts.iter().position(|&part| part == self.focus).unwrap_or(0);
        match parts.get((at + 1) % parts.len()).copied().unwrap_or(Focus::Doc) {
            Focus::Tree => self.focus_tree(),
            other => self.focus = other,
        }
    }

    /// The tree key: hide the tree on the left, close the tree over the document, or show the tree
    /// with the focus.
    fn toggle_tree(&mut self) {
        if self.docked.tree {
            self.tree_open = false;
            if self.focus == Focus::Tree {
                self.focus = Focus::Doc;
            }
        } else if self.focus == Focus::Tree {
            self.focus = Focus::Doc;
            // In the last-reply view, a closed tree stays closed in a wider window too.
            if self.start_view == StartView::Last {
                self.tree_open = false;
            }
        } else {
            self.open_tree();
        }
    }

    /// The annotation key: the focus goes to the card rail or to the annotation list, or back to the
    /// document. Where the window has no rail, the list opens and closes.
    fn toggle_list(&mut self) {
        let beside = self.list_open && self.docked.rail && !self.docked.cards;
        if self.focus == Focus::Rail || beside {
            self.list_open = false;
            if self.focus == Focus::Rail {
                self.focus = Focus::Doc;
            }
            return;
        }
        self.list_open = true;
        self.close_box_only();
        self.focus = Focus::Rail;
        // Start at the first note at the focus of the document, else keep the place.
        let here = self.notes_here().first().copied();
        let order: Vec<u64> =
            self.review().map(|r| r.in_order().iter().map(|a| a.id).collect()).unwrap_or_default();
        match here.and_then(|id| order.iter().position(|&other| other == id)) {
            Some(index) => self.annotation_index = index,
            None => self.annotation_index = self.annotation_index.min(order.len().saturating_sub(1)),
        }
    }

    /// Open the box, or close it when it shows the same view.
    fn toggle_box(&mut self, view: BoxView) {
        if self.boxed == Some(view) {
            self.close_box();
            return;
        }
        self.boxed = Some(view);
        self.start_box = false;
        self.focus = Focus::Box;
        match view {
            BoxView::Replies => self.panel_index = self.selected,
            BoxView::Finder => self.finder = Finder::new(self.tree.file_paths()),
        }
    }

    pub(super) fn close_box(&mut self) {
        self.close_box_only();
        self.focus = Focus::Doc;
    }

    /// Remove the box and forget that it was the start list.
    fn close_box_only(&mut self) {
        self.boxed = None;
        self.start_box = false;
    }

    /// The keys that work the same in the tree, the rail, and the reply list. `None` when the key is
    /// not one of them.
    fn common_key(&mut self, action: Option<Action>) -> Option<Effect> {
        match action? {
            Action::Quit => return Some(self.request_quit()),
            Action::Help => self.help_open = true,
            Action::Reload => {
                self.reload_shown();
                return Some(Effect::Reload);
            }
            Action::Replies => self.toggle_box(BoxView::Replies),
            Action::Annotations => self.toggle_list(),
            Action::Tree if self.root.is_some() => self.toggle_tree(),
            Action::FindFile if self.root.is_some() => self.toggle_box(BoxView::Finder),
            _ => return None,
        }
        Some(Effect::None)
    }

    /// The finder is a text field: letters go into the query.
    fn finder_key(&mut self, key: KeyEvent) -> Effect {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.close_box(),
            KeyCode::Enter => {
                if let Some(path) = self.finder.selected().map(PathBuf::from) {
                    self.close_box();
                    return self.open_path(&path);
                }
            }
            KeyCode::Down => self.finder.move_by(1),
            KeyCode::Up => self.finder.move_by(-1),
            KeyCode::Char('n') if control => self.finder.move_by(1),
            KeyCode::Char('p') if control => self.finder.move_by(-1),
            KeyCode::Backspace => self.finder.backspace(),
            KeyCode::Char(c) if !control => self.finder.type_char(c),
            _ => {}
        }
        Effect::None
    }

    /// The filter of the tree is a text field too, until esc or enter.
    fn tree_filter_key(&mut self, key: KeyEvent) -> Effect {
        match key.code {
            KeyCode::Esc => self.tree.stop_filter(),
            KeyCode::Enter => return self.tree_enter(),
            KeyCode::Down => self.tree.move_by(1),
            KeyCode::Up => self.tree.move_by(-1),
            KeyCode::Backspace => self.tree.backspace(),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => self.tree.type_char(c),
            _ => {}
        }
        Effect::None
    }

    /// `enter` in the tree: open a file, or open or close a directory.
    pub(super) fn tree_enter(&mut self) -> Effect {
        let Some(entry) = self.tree.selected() else { return Effect::None };
        if entry.is_dir {
            self.tree.toggle();
        } else {
            let path = entry.path.clone();
            self.tree.stop_filter();
            self.focus = Focus::Doc;
            return self.open_path(&path);
        }
        Effect::None
    }

    fn tree_key(&mut self, key: KeyEvent) -> Effect {
        if self.tree.filter().is_some() {
            return self.tree_filter_key(key);
        }
        match key.code {
            KeyCode::Enter => return self.tree_enter(),
            // With no document on the screen the tree is the top level, so `esc` quits.
            KeyCode::Esc if self.shown_file().is_none() && self.shown_id.is_none() => {
                return self.request_quit();
            }
            KeyCode::Esc => self.focus = Focus::Doc,
            KeyCode::Tab => self.cycle_focus(),
            _ => {
                let action = self.action_of(key);
                match action {
                    Some(Action::Left) => self.tree.collapse(),
                    Some(Action::Right) => self.tree.expand(),
                    Some(Action::Find) => self.tree.start_filter(),
                    Some(Action::Down) => self.tree.move_by(1),
                    Some(Action::Up) => self.tree.move_by(-1),
                    _ => return self.common_key(action).unwrap_or(Effect::None),
                }
            }
        }
        Effect::None
    }

    /// The keys of the card rail and of the annotation list.
    fn rail_key(&mut self, key: KeyEvent) -> Effect {
        match key.code {
            KeyCode::Enter => self.rail_enter(),
            KeyCode::Esc => {
                // The list stays open only where it stands beside the document.
                if !self.docked.rail || self.docked.cards {
                    self.list_open = false;
                }
                self.focus = Focus::Doc;
            }
            KeyCode::Tab => self.cycle_focus(),
            _ => {
                let action = self.action_of(key);
                match action {
                    Some(Action::Down) => self.rail_move(1),
                    Some(Action::Up) => self.rail_move(-1),
                    Some(Action::Edit) => {
                        if let Some(id) = self.listed_annotation() {
                            self.list_open = false;
                            self.focus = Focus::Doc;
                            return self.begin_edit(id);
                        }
                    }
                    Some(Action::Remove) => self.confirm = self.listed_annotation().map(Confirm::Annotation),
                    _ => return self.common_key(action).unwrap_or(Effect::None),
                }
            }
        }
        Effect::None
    }

    /// Move in the annotation list. On the card rail, the document shows the note of the focused card.
    fn rail_move(&mut self, delta: isize) {
        let last = self.review().map_or(0, |review| review.annotations.len()).saturating_sub(1);
        self.annotation_index = self.annotation_index.saturating_add_signed(delta).min(last);
        if self.docked.cards
            && let Some(start) = self.listed_place()
        {
            self.cursor = start;
            self.snap_cursor();
            self.focus_near(start);
        }
    }

    /// The start of the note under the cursor of the annotation list. `None` for a detached note.
    fn listed_place(&self) -> Option<usize> {
        let id = self.listed_annotation()?;
        let annotation = self.review()?.annotations.iter().find(|a| a.id == id)?;
        (!annotation.detached).then_some(annotation.range.start)
    }

    /// The keys of the centered box.
    fn box_key(&mut self, key: KeyEvent) -> Effect {
        let Some(view) = self.boxed else {
            self.focus = Focus::Doc;
            return self.doc_key(key);
        };
        if view == BoxView::Finder {
            return self.finder_key(key);
        }
        match key.code {
            KeyCode::Enter => return self.replies_enter(),
            // The start list is the way in: `esc` there ends the session.
            KeyCode::Esc if self.start_box => return self.request_quit(),
            KeyCode::Esc => self.close_box(),
            KeyCode::Tab => {}
            _ => {
                let action = self.action_of(key);
                match action {
                    Some(Action::Down) => self.replies_move(1),
                    Some(Action::Up) => self.replies_move(-1),
                    _ => return self.common_key(action).unwrap_or(Effect::None),
                }
            }
        }
        Effect::None
    }

    fn replies_move(&mut self, delta: isize) {
        if let ReplySource::Pick(candidates) = &self.reply_source {
            let last = candidates.len().saturating_sub(1);
            self.picker_index = self.picker_index.saturating_add_signed(delta).min(last);
            return;
        }
        let last = self.replies.len().saturating_sub(1);
        self.panel_index = self.panel_index.saturating_add_signed(delta).min(last);
    }

    /// The id of the annotation under the cursor of the annotation list.
    pub(super) fn listed_annotation(&self) -> Option<u64> {
        self.review()?.in_order().get(self.annotation_index).map(|annotation| annotation.id)
    }

    /// `enter` in the reply list: show the reply, or choose the session.
    pub(super) fn replies_enter(&mut self) -> Effect {
        if let ReplySource::Pick(candidates) = &self.reply_source {
            return if candidates.is_empty() { Effect::None } else { Effect::PickSession(self.picker_index) };
        }
        self.show_reply(self.panel_index);
        self.close_box();
        Effect::None
    }

    /// `enter` on the rail or in the annotation list: go to the note in the document.
    fn rail_enter(&mut self) {
        if self.listed_annotation().is_none() {
            return;
        }
        match self.listed_place() {
            None => self.status = Some("the text of this annotation was not found".to_owned()),
            Some(start) => {
                self.cursor = start;
                self.snap_cursor();
                self.focus_near(start);
                self.list_open = false;
                self.focus = Focus::Doc;
            }
        }
    }

    fn confirm_key(&mut self, key: KeyEvent, what: Confirm) -> Effect {
        match what {
            Confirm::Quit => match key.code {
                KeyCode::Char('s') => {
                    self.confirm = None;
                    self.quit_after = true;
                    let effect = self.request_send(DeliveryMode::Send);
                    if effect == Effect::None {
                        self.quit_after = false;
                    }
                    effect
                }
                // With an unreadable draft the notes are not on disk, so "keep" would be false.
                KeyCode::Char('k') if self.save_error.is_none() => {
                    self.confirm = None;
                    Effect::Quit
                }
                KeyCode::Char('d') => {
                    self.confirm = None;
                    self.quit_after = true;
                    self.discard_review(true)
                }
                KeyCode::Esc => {
                    self.confirm = None;
                    Effect::None
                }
                _ => Effect::None,
            },
            Confirm::Annotation(id) => {
                self.confirm = None;
                if key.code != KeyCode::Char('y') {
                    return Effect::None;
                }
                let Some(reply) = self.shown_id.clone() else { return Effect::None };
                if let Some(review) = self.reviews.get_mut(&reply) {
                    review.remove(id);
                }
                self.sent.remove(&reply);
                let count = self.review().map_or(0, |review| review.annotations.len());
                self.annotation_index = self.annotation_index.min(count.saturating_sub(1));
                Effect::SaveDraft
            }
            Confirm::Discard => {
                self.confirm = None;
                if key.code == KeyCode::Char('y') { self.discard_review(false) } else { Effect::None }
            }
            Confirm::CloseFile => {
                self.confirm = None;
                if key.code == KeyCode::Char('y') {
                    self.close_shown();
                }
                Effect::None
            }
        }
    }

    /// Remove the review of the reply on the screen. The caller removes the draft file.
    fn discard_review(&mut self, all_files: bool) -> Effect {
        let ids: Vec<String> = if all_files {
            self.replies
                .iter()
                .filter(|reply| self.reviews.get(&reply.id).is_some_and(Review::has_unsent))
                .map(|reply| reply.id.clone())
                .collect()
        } else {
            self.shown_id.clone().into_iter().collect()
        };
        for id in ids {
            self.reviews.remove(&id);
            self.sent.remove(&id);
            self.discarded.push(id);
        }
        self.selection = None;
        self.annotation_index = 0;
        self.save_error = None;
        Effect::Discard
    }

    /// `q` and `esc` at the top level: quit, but ask first when the reply on the screen has notes that no send covered.
    fn request_quit(&mut self) -> Effect {
        self.help_open = false;
        if self.unsent_file_count() > 0 {
            self.confirm = Some(Confirm::Quit);
            Effect::None
        } else {
            Effect::Quit
        }
    }

    /// The number of open files (not replies) with notes that no send took.
    pub fn unsent_open_files(&self) -> usize {
        self.replies
            .iter()
            .filter(|reply| {
                self.files.contains_key(&reply.id)
                    && self.reviews.get(&reply.id).is_some_and(Review::has_unsent)
            })
            .count()
    }

    /// Show no document: the files view starts so when no file is open. A reply opens from the list.
    pub fn show_no_document(&mut self) {
        self.shown_id = None;
        self.doc = Doc::parse("");
        self.cache.clear();
        self.top = 0;
        self.cursor = 0;
        self.mode = Mode::Browse;
        self.selection = None;
        self.annotation_index = 0;
    }

    /// The number of open files with notes that no send took.
    pub fn unsent_file_count(&self) -> usize {
        self.replies
            .iter()
            .filter(|reply| self.reviews.get(&reply.id).is_some_and(Review::has_unsent))
            .count()
    }

    /// `S` and `P`: send or paste the export, when there is something to send.
    pub(super) fn request_send(&mut self, mode: DeliveryMode) -> Effect {
        if self.export().is_empty() {
            self.status = Some(self.nothing_to_send());
            return Effect::None;
        }
        Effect::Send(mode)
    }

    /// `M`: write the general note. The note goes first in the export.
    fn begin_note(&mut self) -> Effect {
        let Some(id) = self.shown_id.clone() else { return Effect::None };
        let note = self.reviews.get(&id).map(|review| review.note.clone()).unwrap_or_default();
        self.editor = Some(Editor::general(note));
        self.editing_id = None;
        Effect::SaveDraft
    }

    /// Why there is nothing to send.
    fn nothing_to_send(&self) -> String {
        let any_notes =
            self.replies.iter().filter_map(|r| self.reviews.get(&r.id)).any(|review| !review.is_empty());
        if any_notes {
            "Nothing new to send. Press Y to copy all notes.".to_owned()
        } else {
            "Nothing to send: no annotations yet.".to_owned()
        }
    }

    /// The number of annotations that no send took, in all documents.
    pub fn unsent_count(&self) -> usize {
        self.replies
            .iter()
            .filter_map(|reply| self.reviews.get(&reply.id))
            .map(|review| review.unsent().len())
            .sum()
    }

    /// True when a document has a general note that no send took.
    pub fn has_unsent_note(&self) -> bool {
        self.replies
            .iter()
            .filter_map(|reply| self.reviews.get(&reply.id))
            .any(|review| !review.note.trim().is_empty())
    }

    /// The number of annotations of the reply on the screen.
    pub fn annotation_count(&self) -> usize {
        self.replies
            .iter()
            .filter_map(|reply| self.reviews.get(&reply.id))
            .map(|review| review.annotations.len())
            .sum()
    }

    /// The Markdown that a send gives the agent: the general note, then the annotations that no
    /// send took yet. Empty when there is nothing to send.
    pub fn export(&self) -> String {
        self.export_of(false)
    }

    /// The Markdown of all annotations, also the ones that a send took. `Y` copies this.
    pub fn export_all(&self) -> String {
        self.export_of(true)
    }

    /// One message for all documents: the general notes, then one section for each document with
    /// notes: the replies (newest first), then the files in the order in which they were opened.
    fn export_of(&self, all: bool) -> String {
        struct Part<'a> {
            name: String,
            source: String,
            annotations: Vec<&'a Annotation>,
        }
        let mut notes: Vec<&str> = Vec::new();
        let mut parts: Vec<Part<'_>> = Vec::new();
        for reply in &self.replies {
            let Some(review) = self.reviews.get(&reply.id) else { continue };
            let note = review.note.trim();
            if !note.is_empty() {
                notes.push(note);
            }
            let annotations = if all { review.in_order() } else { review.unsent() };
            if annotations.is_empty() {
                continue;
            }
            let name = self.name_of(&reply.id);
            let source = if self.shown_id.as_deref() == Some(reply.id.as_str()) {
                self.doc.source.clone()
            } else {
                sanitize(&reply.text)
            };
            parts.push(Part { name, source, annotations });
        }
        let line_ofs: Vec<Box<dyn Fn(usize) -> usize + '_>> = parts
            .iter()
            .map(|part| {
                let source = part.source.as_str();
                Box::new(move |offset: usize| {
                    source.get(..offset).map_or(1, |head| head.matches('\n').count() + 1)
                }) as Box<dyn Fn(usize) -> usize + '_>
            })
            .collect();
        let sections: Vec<Section<'_>> = parts
            .iter()
            .zip(&line_ofs)
            .map(|(part, line_of)| Section {
                name: &part.name,
                source: &part.source,
                line_of: line_of.as_ref(),
                annotations: part.annotations.clone(),
            })
            .collect();
        crate::review::export(&notes.join("\n\n"), &sections)
    }

    /// The name of a document in the export and in its draft: the relative path of a file, `last reply`
    /// for the newest reply, else `earlier reply (HH:MM)` in the local time.
    fn name_of(&self, id: &str) -> String {
        if let Some(file) = self.files.get(id) {
            return file.relative.display().to_string();
        }
        let Some(position) = self.replies.iter().position(|reply| reply.id == id) else {
            return id.to_owned();
        };
        let time = self
            .replies
            .get(position)
            .and_then(|reply| reply.at.as_deref())
            .and_then(|at| at.parse::<jiff::Timestamp>().ok())
            .map(|time| time.to_zoned(self.zone.clone()).strftime("%H:%M").to_string());
        match (position, time) {
            (0, _) => "last reply".to_owned(),
            (_, Some(time)) => format!("earlier reply ({time})"),
            (_, None) => "earlier reply".to_owned(),
        }
    }

    /// A successful send closes Annorex. The caller saves the drafts first.
    pub fn close_after_send(&mut self) {
        self.exit = true;
    }

    /// The reply counts as sent at `at`, and the general note is used up.
    pub fn finish_send(&mut self, at: &str) {
        for reply in &self.replies {
            if let Some(review) = self.reviews.get_mut(&reply.id) {
                review.note.clear();
                review.mark_sent(at);
                self.sent.insert(reply.id.clone(), at.to_owned());
            }
        }
    }

    /// The ids of the documents whose notes the last discard removed. The caller removes their drafts.
    pub fn take_discarded(&mut self) -> Vec<String> {
        std::mem::take(&mut self.discarded)
    }

    /// End the viewer after the send or the discard that the quit question started.
    pub fn quit_if_pending(&mut self) {
        if self.quit_after {
            self.exit = true;
        }
    }

    /// The send failed: stay in the viewer.
    pub fn cancel_pending_quit(&mut self) {
        self.quit_after = false;
    }

    /// True when the viewer must end now.
    pub fn should_exit(&self) -> bool {
        self.exit
    }

    /// `v` and `V`: start a selection. When the same kind is on, the key ends it and opens the action
    /// strip. When the strip was open already, the key removes the selection.
    fn toggle_selection(&mut self, lines: bool, strip_was_open: bool) {
        let anchor = self.cursor;
        self.selection = match (self.selection, lines) {
            // The key of the selection ends it: the selection stays, and the action strip opens. A
            // press with the strip already open removes the selection.
            (Some(selection @ Selection::Chars { .. }), false)
            | (Some(selection @ Selection::Lines { .. }), true)
                if !strip_was_open =>
            {
                self.bar_open = true;
                Some(selection)
            }
            (Some(Selection::Chars { .. }), false) | (Some(Selection::Lines { .. }), true) => None,
            (_, false) => Some(Selection::Chars { anchor }),
            (_, true) => Some(Selection::Lines { anchor }),
        };
    }

    /// The bytes of the selection, or `None` without a selection.
    pub(crate) fn selection_range(&self) -> Option<Range<usize>> {
        match self.selection? {
            Selection::Chars { anchor } => {
                let (low, high) = (anchor.min(self.cursor), anchor.max(self.cursor));
                Some(low..self.doc.char_end(high))
            }
            Selection::Lines { anchor } => {
                let (a, b) = (self.doc.locate(anchor).0, self.cursor_line());
                let (first, last) = (a.min(b), a.max(b));
                Some(self.doc.lines.get(first)?.source.start..self.doc.lines.get(last)?.source.end)
            }
        }
    }

    /// The range of an annotation: the focused block in block mode; in text mode the selection, else
    /// the `DocLine` of the cursor.
    pub(crate) fn action_range(&self) -> Option<Range<usize>> {
        match self.mode {
            Mode::Block(index) => return self.doc.blocks.get(index).map(|block| block.range.clone()),
            Mode::Browse => return None,
            Mode::Text => {}
        }
        self.selection_range()
            .or_else(|| self.doc.lines.get(self.cursor_line()).map(|line| line.source.clone()))
    }

    /// The footer message when a note key finds no range: ask for a block when there are blocks
    /// but none has the focus.
    fn no_range_message(&self) -> &'static str {
        if self.mode == Mode::Browse && !self.doc.blocks.is_empty() {
            CHOOSE_A_BLOCK
        } else {
            NOTHING_TO_ANNOTATE
        }
    }

    /// True, with the footer message, when block mode has no focus and the keys of a note cannot act.
    fn needs_a_block(&mut self) -> bool {
        let needs = self.mode == Mode::Browse && !self.doc.blocks.is_empty();
        if needs {
            self.status = Some(CHOOSE_A_BLOCK.to_owned());
        }
        needs
    }

    pub(super) fn begin_new(&mut self, kind: Kind) -> Effect {
        if self.shown_id.is_none() {
            return Effect::None;
        }
        let Some(range) = self.action_range().filter(|range| !range.is_empty()) else {
            self.status = Some(self.no_range_message().to_owned());
            return Effect::None;
        };
        self.editor = Some(Editor::new(kind, range));
        self.editing_id = None;
        Effect::SaveDraft
    }

    /// `o`: a "looks good" note on the current range. No editor opens.
    pub fn add_looks_good(&mut self) -> Effect {
        self.add_instant(Kind::LooksGood)
    }

    /// `o` and `d`: a note without text on the current range. It is saved at once and no box opens.
    /// The strip closes and the selection ends.
    pub(super) fn add_instant(&mut self, kind: Kind) -> Effect {
        let Some(reply) = self.shown_id.clone() else { return Effect::None };
        let Some(range) = self.action_range().filter(|range| !range.is_empty()) else {
            self.status = Some(self.no_range_message().to_owned());
            return Effect::None;
        };
        let review = self.reviews.entry(reply.clone()).or_default();
        if review.annotations.iter().any(|a| {
            a.sent_at.is_none()
                && a.kind == kind
                && a.range == range
                && a.label.is_none()
                && a.note.is_empty()
        }) {
            self.status = Some("Already saved.".to_owned());
        } else {
            review.add(&self.doc.source, kind, range);
            self.sent.remove(&reply);
        }
        self.bar_open = false;
        self.selection = None;
        Effect::SaveDraft
    }

    /// `L`: the editor of the annotation at the cursor, else a new comment.
    fn begin_label(&mut self) -> Effect {
        if self.needs_a_block() {
            return Effect::None;
        }
        let id = self.review().and_then(|review| {
            review.at(self.cursor).into_iter().map(|a| a.id).find(|id| self.takes_text(*id))
        });
        match id {
            Some(id) => self.begin_edit(id),
            None => self.begin_new(Kind::Comment),
        }
    }

    /// The ids of the notes at the focus, in document order: the notes on the focused block in block
    /// mode, the notes at the cursor in text mode. Detached notes are left out.
    pub(super) fn notes_here(&self) -> Vec<u64> {
        let Some(review) = self.review() else { return Vec::new() };
        let block = self.focus_range();
        if self.mode == Mode::Browse {
            return Vec::new();
        }
        review
            .in_order()
            .into_iter()
            .filter(|a| !a.detached)
            .filter(|a| match &block {
                Some(block) => overlaps(&a.range, block),
                None => a.range.contains(&self.cursor),
            })
            .map(|a| a.id)
            .collect()
    }

    /// The edit or the remove key in the document: act on the note at the focus. With several
    /// notes, open the numbered pick.
    fn note_here(&mut self, remove: bool) -> Effect {
        if self.needs_a_block() {
            return Effect::None;
        }
        let mut ids = self.notes_here();
        if !remove {
            // Only a note with text can be edited. Say why when no note here can.
            let editable: Vec<u64> = ids.iter().copied().filter(|id| self.takes_text(*id)).collect();
            if let (true, Some(first)) = (editable.is_empty(), ids.first().copied()) {
                return self.begin_edit(first);
            }
            ids = editable;
        }
        match ids.as_slice() {
            [] => self.status = Some("no note here".to_owned()),
            [id] => return self.act_on_note(*id, remove),
            _ => {
                ids.truncate(MAX_PICK);
                self.pick = Some(ids);
                self.pick_removes = remove;
            }
        }
        Effect::None
    }

    /// Edit a note, or ask before its removal.
    pub(super) fn act_on_note(&mut self, id: u64, remove: bool) -> Effect {
        if remove {
            self.confirm = Some(Confirm::Annotation(id));
            Effect::None
        } else {
            self.begin_edit(id)
        }
    }

    /// A key while the numbered pick is open. `1` to `9` choose a note, `esc` closes the pick, a
    /// number that is not in the list does nothing. Another key closes the pick and then acts as
    /// usual: the result is `None` then.
    fn pick_key(&mut self, key: KeyEvent) -> Option<Effect> {
        let ids = self.pick.take()?;
        match key.code {
            KeyCode::Esc => Some(Effect::None),
            KeyCode::Char(c @ '1'..='9') => {
                let chosen = c.to_digit(10).and_then(|n| ids.get(usize::try_from(n).ok()? - 1)).copied();
                if let Some(id) = chosen {
                    return Some(self.act_on_note(id, self.pick_removes));
                }
                self.pick = Some(ids);
                Some(Effect::None)
            }
            _ => None,
        }
    }

    /// Whether the note with this id can have text: a comment or a replace.
    fn takes_text(&self, id: u64) -> bool {
        self.review()
            .and_then(|review| review.annotations.iter().find(|a| a.id == id))
            .is_some_and(|a| matches!(a.kind, Kind::Comment | Kind::Replace))
    }

    pub(super) fn begin_edit(&mut self, id: u64) -> Effect {
        let Some(annotation) =
            self.review().and_then(|review| review.annotations.iter().find(|a| a.id == id))
        else {
            return Effect::None;
        };
        match annotation.kind {
            Kind::LooksGood => {
                self.status = Some("Looks good has no text.".to_owned());
                return Effect::None;
            }
            Kind::Delete => {
                self.status = Some("Remove has no text.".to_owned());
                return Effect::None;
            }
            Kind::Comment | Kind::Replace => {}
        }
        let text = if annotation.kind == Kind::Replace {
            annotation.replacement.clone().unwrap_or_default()
        } else {
            annotation.note.clone()
        };
        self.editor = Some(Editor {
            kind: annotation.kind,
            range: annotation.range.clone(),
            text,
            label: annotation.label.clone(),
            scroll: None,
            rows: 0,
            general: false,
        });
        self.editing_id = Some(id);
        Effect::SaveDraft
    }

    fn editor_key(&mut self, key: KeyEvent) -> Effect {
        let Some(editor) = &mut self.editor else { return Effect::None };
        let result = editor.handle_key(key);
        self.editor_done(result)
    }

    /// Act on the result of the editor: a key, or a click on `save` or `cancel`.
    pub(super) fn editor_done(&mut self, result: EditorResult) -> Effect {
        let Some(editor) = &self.editor else { return Effect::None };
        match result {
            EditorResult::Editing => Effect::SaveDraft,
            EditorResult::Cancel => {
                self.editor = None;
                self.editing_id = None;
                Effect::SaveDraft
            }
            EditorResult::Save => {
                if let Some(problem) = editor.problem() {
                    self.status = Some(problem.to_owned());
                    return Effect::None;
                }
                if editor.general {
                    return self.commit_note();
                }
                self.commit_editor();
                Effect::SaveDraft
            }
        }
    }

    /// Store the general note and ask for the send.
    fn commit_note(&mut self) -> Effect {
        let (Some(editor), Some(reply)) = (self.editor.take(), self.shown_id.clone()) else {
            return Effect::None;
        };
        let review = self.reviews.entry(reply.clone()).or_default();
        editor.text.trim().clone_into(&mut review.note);
        self.sent.remove(&reply);
        if self.export().is_empty() {
            self.status = Some(self.nothing_to_send());
            return Effect::SaveDraft;
        }
        Effect::SendWithNote
    }

    /// Put the editor into the review: a new annotation, or the change of an old one.
    fn commit_editor(&mut self) {
        let (Some(editor), Some(reply)) = (self.editor.take(), self.shown_id.clone()) else {
            return;
        };
        let review = self.reviews.entry(reply.clone()).or_default();
        let existing = self.editing_id.take().filter(|id| review.annotations.iter().any(|a| a.id == *id));
        // A new note, or an edit, that makes an exact copy of another unsent note is refused.
        let same_text = |a: &Annotation| {
            if editor.kind == Kind::Replace {
                a.replacement.as_ref() == Some(&editor.text)
            } else {
                a.note == editor.text
            }
        };
        if review.annotations.iter().any(|a| {
            Some(a.id) != existing
                && a.sent_at.is_none()
                && a.kind == editor.kind
                && a.range == editor.range
                && a.label == editor.label
                && same_text(a)
        }) {
            if existing.is_some() {
                // An edit: keep the editor and its text, so the user can change it.
                self.status = Some("Another note has this text.".to_owned());
                self.editor = Some(editor);
                self.editing_id = existing;
            } else {
                self.status = Some("Already saved.".to_owned());
                self.selection = None;
            }
            return;
        }
        let Editor { kind, range, text, label, .. } = editor;
        let id = existing.unwrap_or_else(|| review.add(&self.doc.source, kind, range));
        review.update(id, |annotation: &mut Annotation| {
            annotation.label = label;
            if annotation.kind == Kind::Replace {
                annotation.replacement = Some(text);
            } else {
                annotation.note = text;
            }
        });
        self.sent.remove(&reply);
        self.selection = None;
    }

    fn find_key(&mut self, key: KeyEvent) {
        let Some(input) = &mut self.find_input else { return };
        match key.code {
            KeyCode::Esc => self.find_input = None,
            KeyCode::Enter => {
                let query = std::mem::take(input);
                self.find_input = None;
                if !query.is_empty() {
                    self.query = Some(query);
                    self.find_next(true, true);
                }
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => input.push(c),
            _ => {}
        }
    }

    /// Move the cursor to the next (or previous) line that contains the query. The search wraps.
    /// A new search also looks at the line of the cursor.
    fn find_next(&mut self, forward: bool, new_search: bool) {
        let Some(query) = self.query.clone() else { return };
        let count = self.doc.lines.len();
        if count == 0 {
            self.status = Some("no match".to_owned());
            return;
        }
        let start = self.cursor_line();
        let found = (0..count)
            .map(|step| {
                let step = if new_search { step } else { step + 1 };
                if forward { (start + step) % count } else { (start + count * 2 - step) % count }
            })
            .find(|&index| {
                self.doc.lines.get(index).is_some_and(|line| contains_ignore_case(&line.plain(), &query))
            });
        match found {
            Some(index) => self.find_at(index, &query),
            None => self.status = Some("no match".to_owned()),
        }
    }

    /// Put the cursor on the first match of `query` in the `DocLine` `index`, in text mode.
    fn find_at(&mut self, index: usize, query: &str) {
        let plain = self.doc.lines.get(index).map(crate::doc::DocLine::plain).unwrap_or_default();
        let first = match_ranges(&plain, query).first().map_or(0, |range| range.start);
        let column = plain.chars().take(first).filter_map(unicode_width::UnicodeWidthChar::width).sum();
        if let Some(stop) = self.doc.stop_near(index, column).or_else(|| self.line_start(index)) {
            self.cursor = stop;
            self.mode = Mode::Text;
        }
    }

    fn set_cursor_line(&mut self, index: usize) {
        if let Some(start) = self.line_start(index) {
            self.cursor = start;
        }
    }

    /// Move the cursor to the previous or next stop of its line.
    fn step(&mut self, forward: bool) {
        let stops = self.doc.stops(self.cursor_line());
        let Some(at) = stops.iter().position(|&stop| stop == self.cursor) else { return };
        let target = if forward { at + 1 } else { at.wrapping_sub(1) };
        if let Some(&stop) = stops.get(target) {
            self.cursor = stop;
        }
    }

    /// Move the cursor to the first or the last stop of its line.
    fn line_edge(&mut self, last: bool) {
        let stops = self.doc.stops(self.cursor_line());
        if let Some(&stop) = if last { stops.last() } else { stops.first() } {
            self.cursor = stop;
        }
    }

    /// Move the cursor to the next word start, or to the start of this word or the word before.
    fn word(&mut self, forward: bool) {
        let mut starts = Vec::new();
        for line in 0..self.doc.lines.len() {
            let mut in_word = false;
            for stop in self.doc.stops(line) {
                let word =
                    self.doc.source.get(stop..).and_then(|rest| rest.chars().next()).is_some_and(is_word);
                if word && !in_word {
                    starts.push(stop);
                }
                in_word = word;
            }
        }
        let target = if forward {
            starts.iter().find(|&&start| start > self.cursor)
        } else {
            starts.iter().rev().find(|&&start| start < self.cursor)
        };
        if let Some(&start) = target {
            self.cursor = start;
        }
    }

    /// Move the cursor by `delta` text lines, to the same display column. Blank lines do not count.
    fn move_cursor(&mut self, delta: isize) {
        let text: Vec<usize> =
            self.doc.lines.iter().enumerate().filter(|(_, line)| !line.is_blank()).map(|(i, _)| i).collect();
        let (line, column) = self.doc.locate(self.cursor);
        let Some(position) = text.iter().position(|&index| index == line) else { return };
        let target = position.saturating_add_signed(delta).min(text.len().saturating_sub(1));
        let Some(&index) = text.get(target) else { return };
        if let Some(stop) = self.doc.stop_near(index, column).or_else(|| self.line_start(index)) {
            self.cursor = stop;
        }
    }
}

/// True when two source ranges share a byte.
fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// A character of a word: a letter, a digit, or an underscore.
pub(super) fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn signed(value: usize) -> isize {
    isize::try_from(value).unwrap_or(isize::MAX)
}

/// Case-insensitive search, character by character.
pub(crate) fn contains_ignore_case(text: &str, query: &str) -> bool {
    !match_ranges(text, query).is_empty()
}

/// The character ranges of each match of `query` in `text`, without overlap.
pub(crate) fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let text: Vec<char> = text.chars().map(fold).collect();
    let query: Vec<char> = query.chars().map(fold).collect();
    let mut out = Vec::new();
    if query.is_empty() || query.len() > text.len() {
        return out;
    }
    let mut index = 0;
    while index + query.len() <= text.len() {
        if text.get(index..index + query.len()) == Some(query.as_slice()) {
            out.push(index..index + query.len());
            index += query.len();
        } else {
            index += 1;
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use crate::ui::{AgentState, Screen};

    const NOTICE: &str = "Annorex 9.9.9 is available. Run: annorex update";

    fn app() -> App {
        let state = AgentState { agent: None, safety: Safety::Clear { state: None } };
        App::new(state, Screen::Viewer, Vec::new(), &Config::default())
    }

    fn notice_sent(app: &mut App) {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(NOTICE.to_owned()).expect("send");
        app.start_update_check(receiver);
        assert!(app.poll_background(), "the notice changes the screen");
    }

    #[test]
    fn the_update_notice_shows_in_the_banner() {
        let mut app = app();
        assert_eq!(app.banner(), None);
        notice_sent(&mut app);
        assert_eq!(app.banner().as_deref(), Some(NOTICE));
    }

    #[test]
    fn the_update_notice_comes_last_after_the_file_banner() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join("a.txt"), "alpha\n").expect("write");
        let mut app = app().with_root(dir.path().to_path_buf());
        app.open_file(crate::files::load(dir.path(), Path::new("a.txt")).expect("load"));
        std::fs::write(dir.path().join("a.txt"), "changed\n").expect("change");
        app.file_changed(&Change { path: PathBuf::from("a.txt"), kind: ChangeKind::Modified });
        notice_sent(&mut app);
        assert_eq!(
            app.banner().as_deref(),
            Some(format!("a.txt changed on disk. Press r to read it again.  |  {NOTICE}").as_str())
        );
    }

    #[test]
    fn a_new_app_keeps_the_update_check_of_the_old_app() {
        let mut old = app();
        notice_sent(&mut old);
        let mut fresh = app();
        fresh.keep_update_check(&mut old);
        assert_eq!(fresh.banner().as_deref(), Some(NOTICE));
    }
}
