//! The state of the viewer and its keys.

use std::collections::HashMap;
use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::editor::{Editor, EditorResult};
use crate::agent::Agent;
use crate::config::Config;
use crate::doc::Doc;
use crate::draft::{Draft, DraftKey};
use crate::keys::Action;
use crate::payload::sanitize;
use crate::replies::{Candidate, Reply};
use crate::review::{Annotation, Kind, Review};
use crate::safety::Safety;
use crate::wrap::WrapCache;

/// The agent in the target pane and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentState {
    /// `None` when the pane does not run a supported agent.
    pub agent: Option<Agent>,
    pub safety: Safety,
}

impl AgentState {
    /// For example `claude (idle)`, `pi (working…)`, `opencode (waiting for permission)`.
    pub fn header_text(&self) -> String {
        let Some(agent) = self.agent else { return "no agent".to_owned() };
        let name = agent.host().label();
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

/// What the caller must do after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
    Reload,
    ToggleZoom,
    PickSession(usize),
    /// The review or the editor of the current reply changed. The caller saves the draft.
    SaveDraft,
}

/// Where the keys go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Doc,
    Panel,
}

/// What the side panel lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PanelView {
    Replies,
    Annotations,
}

/// The text that the user marked with `v` or `V`. The anchor is where the marking started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selection {
    Chars { anchor: usize },
    Lines { anchor: usize },
}

/// The state of the viewer.
#[derive(Debug)]
pub struct App {
    pub state: AgentState,
    pub screen: Screen,
    pub(crate) replies: Vec<Reply>,
    pub(crate) selected: usize,
    pub(crate) doc: Doc,
    pub(crate) cache: WrapCache,
    /// The `DocLine` at the top of the view. A resize keeps it.
    pub(crate) top: usize,
    /// A byte offset in the source of the document.
    pub(crate) cursor: usize,
    pub(crate) panel_open: bool,
    pub(crate) panel_index: usize,
    pub(crate) picker_index: usize,
    pub(crate) focus: Focus,
    /// The text of the find input while the user types it.
    pub(crate) find_input: Option<String>,
    pub(crate) query: Option<String>,
    /// A message for the footer.
    pub(crate) status: Option<String>,
    /// The height of the document area in the last frame.
    pub(crate) page: usize,
    /// True when the last frame showed the panel over the document.
    pub(crate) overlay: bool,
    /// The time zone of the times in the reply list. The default is the system zone.
    pub(crate) zone: jiff::tz::TimeZone,
    pub(crate) config: Config,
    /// The annotations of each reply, by reply id.
    pub(crate) reviews: HashMap<String, Review>,
    /// The time of the last send of each reply that has one. A change removes the entry.
    pub(crate) sent: HashMap<String, String>,
    pub(crate) selection: Option<Selection>,
    pub(crate) editor: Option<Editor>,
    /// The annotation that the editor changes. `None` for a new annotation.
    pub(crate) editing_id: Option<u64>,
    /// Editors of replies that are not on the screen.
    parked: HashMap<String, crate::draft::Editing>,
    pub(crate) panel_view: PanelView,
    pub(crate) annotation_index: usize,
    /// The annotation that waits for `y` or `n`.
    pub(crate) confirm: Option<u64>,
    /// The id of the reply in `doc`.
    shown_id: Option<String>,
    /// Why the last save of the draft failed.
    pub(crate) save_error: Option<String>,
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
            panel_open: false,
            panel_index: 0,
            picker_index: 0,
            focus: Focus::Doc,
            find_input: None,
            query: None,
            status: None,
            page: 20,
            overlay: false,
            zone: jiff::tz::TimeZone::system(),
            config: config.clone(),
            reviews: HashMap::new(),
            sent: HashMap::new(),
            selection: None,
            editor: None,
            editing_id: None,
            parked: HashMap::new(),
            panel_view: PanelView::Replies,
            annotation_index: 0,
            confirm: None,
            shown_id: None,
            save_error: None,
        };
        if !replies.is_empty() {
            app.replies = replies;
            app.show_reply(0);
        }
        app
    }

    /// The document of the selected reply.
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
    fn line_start(&self, index: usize) -> Option<usize> {
        let line = self.doc.lines.get(index)?;
        Some(self.doc.stops(index).first().copied().unwrap_or(line.source.start))
    }

    /// Put `doc_line` at the top of the view and the cursor on it.
    pub fn jump_to_line(&mut self, doc_line: usize) {
        if let Some(start) = self.line_start(doc_line) {
            self.top = doc_line;
            self.cursor = start;
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
        let keep = current
            .and_then(|id| replies.iter().position(|reply| reply.id == id))
            .filter(|&index| self.selected > 0 || index == 0);
        let (top, cursor) = (self.top, self.cursor);
        self.replies = replies;
        self.screen = Screen::Viewer;
        match keep {
            Some(index) => {
                self.show_reply(index);
                self.top = top.min(self.doc.lines.len().saturating_sub(1));
                self.cursor = cursor.min(self.doc.source.len());
                self.snap_cursor();
            }
            None => self.show_reply(0),
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
        self.screen = Screen::Picker(candidates);
        self.picker_index = 0;
    }

    pub fn set_error(&mut self, message: String) {
        self.screen = Screen::Error(message);
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
        self.doc = Doc::parse(&reply.text);
        self.cache.clear();
        self.top = 0;
        let first = self.doc.lines.iter().position(|line| !line.is_blank());
        self.cursor = first.and_then(|index| self.line_start(index)).unwrap_or(0);
        if self.shown_id.as_deref() != Some(id.as_str()) {
            // Another reply: its editor was parked, and a selection or a question do not carry over.
            if let (Some(editor), Some(old)) = (self.editor.take(), self.shown_id.take()) {
                self.parked.insert(old, editor.to_editing());
            }
            self.editing_id = None;
            self.selection = None;
            self.confirm = None;
            self.annotation_index = 0;
            self.shown_id = Some(id.clone());
            if let Some(editing) = self.parked.remove(&id) {
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
        let id = self.shown_id.clone()?;
        let key = DraftKey { agent, session: session.to_owned(), document: id.clone() };
        let mut draft = Draft::new(key, "last reply");
        draft.review = self.reviews.get(&id).cloned().unwrap_or_default();
        draft.editing = self.editor.as_ref().map(Editor::to_editing);
        draft.sent_at = self.sent.get(&id).cloned();
        Some(draft)
    }

    /// Put a saved draft back: the review (checked against the reply), the send time, and the
    /// editor that was open.
    pub fn restore(&mut self, reply_id: &str, draft: Draft) {
        let Draft { mut review, editing, sent_at, .. } = draft;
        if let Some(reply) = self.replies.iter().find(|reply| reply.id == reply_id) {
            review.reanchor(&sanitize(&reply.text));
        }
        self.reviews.insert(reply_id.to_owned(), review);
        match sent_at {
            Some(at) => self.sent.insert(reply_id.to_owned(), at),
            None => self.sent.remove(reply_id),
        };
        if let Some(editing) = editing {
            if self.shown_id.as_deref() == Some(reply_id) {
                self.editor = Some(Editor::from_editing(editing));
                self.editing_id = None;
            } else {
                self.parked.insert(reply_id.to_owned(), editing);
            }
        }
    }

    /// Record a send or a paste of the reply on the screen.
    pub fn mark_sent(&mut self, at: String) {
        if let Some(id) = self.shown_id.clone() {
            self.sent.insert(id, at);
        }
    }

    /// The numbers (1 is the newest) of the replies with annotations that no send has covered.
    pub fn unsent_replies(&self) -> Vec<usize> {
        self.replies
            .iter()
            .enumerate()
            .filter(|(_, reply)| {
                self.reviews.get(&reply.id).is_some_and(|review| !review.is_empty())
                    && !self.sent.contains_key(&reply.id)
            })
            .map(|(index, _)| index + 1)
            .collect()
    }

    /// Handle one key.
    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if key.kind != KeyEventKind::Press {
            return Effect::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Effect::Quit;
        }
        self.status = None;
        if self.editor.is_some() {
            return self.editor_key(key);
        }
        if let Some(id) = self.confirm {
            return self.confirm_key(key, id);
        }
        if self.find_input.is_some() {
            self.find_key(key);
            return Effect::None;
        }
        match &self.screen {
            Screen::Error(_) => match key.code {
                KeyCode::Char('r') => Effect::Reload,
                KeyCode::Char('q') => Effect::Quit,
                _ => Effect::None,
            },
            Screen::Picker(candidates) => {
                let count = candidates.len();
                match key.code {
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
                Focus::Panel => self.panel_key(key),
                Focus::Doc => self.doc_key(key),
            },
        }
    }

    /// The action of a key. The arrow keys always move, whatever the config says.
    fn action_of(&self, key: KeyEvent) -> Option<Action> {
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
        let half = signed((self.page / 2).max(1));
        match key.code {
            KeyCode::PageDown => self.move_cursor(signed(self.page.max(1))),
            KeyCode::PageUp => self.move_cursor(-signed(self.page.max(1))),
            KeyCode::Home => self.set_cursor_line(0),
            KeyCode::End => self.set_cursor_line(self.doc.lines.len().saturating_sub(1)),
            KeyCode::Tab if self.panel_open => self.focus = Focus::Panel,
            KeyCode::Esc => self.selection = None,
            _ => match self.action_of(key) {
                Some(Action::Quit) => return Effect::Quit,
                Some(Action::Reload) => return Effect::Reload,
                Some(Action::Zoom) => return Effect::ToggleZoom,
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
                Some(Action::FindNext) => self.find_next(true, false),
                Some(Action::FindPrev) => self.find_next(false, false),
                Some(Action::Replies) => self.toggle_panel(PanelView::Replies),
                Some(Action::Annotations) => self.toggle_panel(PanelView::Annotations),
                Some(Action::Select) => self.toggle_selection(false),
                Some(Action::SelectLines) => self.toggle_selection(true),
                Some(Action::Comment) => return self.begin_new(Kind::Comment),
                Some(Action::Delete) => return self.begin_new(Kind::Delete),
                Some(Action::Replace) => return self.begin_new(Kind::Replace),
                Some(Action::Label) => return self.begin_label(),
                _ => {}
            },
        }
        Effect::None
    }

    fn toggle_panel(&mut self, view: PanelView) {
        if self.panel_open && self.panel_view == view {
            self.panel_open = false;
            self.focus = Focus::Doc;
            return;
        }
        self.panel_open = true;
        self.panel_view = view;
        self.focus = Focus::Panel;
        match view {
            PanelView::Replies => self.panel_index = self.selected,
            PanelView::Annotations => self.annotation_index = 0,
        }
    }

    fn close_panel_if_overlay(&mut self) {
        self.focus = Focus::Doc;
        if self.overlay {
            self.panel_open = false;
        }
    }

    fn panel_key(&mut self, key: KeyEvent) -> Effect {
        match key.code {
            KeyCode::Enter => return self.panel_enter(),
            KeyCode::Esc | KeyCode::Tab => self.close_panel_if_overlay(),
            _ => match self.action_of(key) {
                Some(Action::Quit) => return Effect::Quit,
                Some(Action::Reload) => return Effect::Reload,
                Some(Action::Zoom) => return Effect::ToggleZoom,
                Some(Action::Down) => self.panel_move(1),
                Some(Action::Up) => self.panel_move(-1),
                Some(Action::Replies) => self.toggle_panel(PanelView::Replies),
                Some(Action::Annotations) => self.toggle_panel(PanelView::Annotations),
                Some(Action::Edit) if self.panel_view == PanelView::Annotations => {
                    if let Some(id) = self.listed_annotation() {
                        self.panel_open = false;
                        self.focus = Focus::Doc;
                        return self.begin_edit(id);
                    }
                }
                Some(Action::Remove) if self.panel_view == PanelView::Annotations => {
                    self.confirm = self.listed_annotation();
                }
                _ => {}
            },
        }
        Effect::None
    }

    fn panel_move(&mut self, delta: isize) {
        match self.panel_view {
            PanelView::Replies => {
                let last = self.replies.len().saturating_sub(1);
                self.panel_index = self.panel_index.saturating_add_signed(delta).min(last);
            }
            PanelView::Annotations => {
                let last = self.review().map_or(0, |review| review.annotations.len()).saturating_sub(1);
                self.annotation_index = self.annotation_index.saturating_add_signed(delta).min(last);
            }
        }
    }

    /// The id of the annotation under the cursor of the annotation list.
    fn listed_annotation(&self) -> Option<u64> {
        self.review()?.in_order().get(self.annotation_index).map(|annotation| annotation.id)
    }

    fn panel_enter(&mut self) -> Effect {
        match self.panel_view {
            PanelView::Replies => {
                self.show_reply(self.panel_index);
                self.close_panel_if_overlay();
            }
            PanelView::Annotations => {
                let range = self.listed_annotation().and_then(|id| {
                    self.review()?
                        .annotations
                        .iter()
                        .find(|a| a.id == id)
                        .map(|a| (a.range.start, a.detached))
                });
                match range {
                    Some((_, true)) => {
                        self.status = Some("the text of this annotation was not found".to_owned());
                    }
                    Some((start, false)) => {
                        self.cursor = start;
                        self.snap_cursor();
                        self.panel_open = false;
                        self.focus = Focus::Doc;
                    }
                    None => {}
                }
            }
        }
        Effect::None
    }

    fn confirm_key(&mut self, key: KeyEvent, id: u64) -> Effect {
        self.confirm = None;
        if key.code != KeyCode::Char('y') {
            return Effect::None;
        }
        let Some(reply) = self.shown_id.clone() else { return Effect::None };
        if let Some(review) = self.reviews.get_mut(&reply) {
            review.remove(id);
        }
        self.sent.remove(&reply);
        self.annotation_index =
            self.annotation_index.min(self.review().map_or(0, |r| r.annotations.len()).saturating_sub(1));
        Effect::SaveDraft
    }

    /// `v` and `V`: start a selection, or end it when the same kind is on.
    fn toggle_selection(&mut self, lines: bool) {
        let anchor = self.cursor;
        self.selection = match (self.selection, lines) {
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

    /// The range of an annotation: the selection, else the `DocLine` of the cursor.
    fn action_range(&self) -> Option<Range<usize>> {
        self.selection_range()
            .or_else(|| self.doc.lines.get(self.cursor_line()).map(|line| line.source.clone()))
    }

    fn begin_new(&mut self, kind: Kind) -> Effect {
        if self.shown_id.is_none() {
            return Effect::None;
        }
        let Some(range) = self.action_range().filter(|range| !range.is_empty()) else {
            self.status = Some("there is no text here to annotate".to_owned());
            return Effect::None;
        };
        self.editor = Some(Editor::new(kind, range));
        self.editing_id = None;
        Effect::SaveDraft
    }

    /// `L`: the editor of the annotation at the cursor, else a new comment.
    fn begin_label(&mut self) -> Effect {
        let id = self.review().and_then(|review| review.at(self.cursor).first().map(|a| a.id));
        match id {
            Some(id) => self.begin_edit(id),
            None => self.begin_new(Kind::Comment),
        }
    }

    fn begin_edit(&mut self, id: u64) -> Effect {
        let Some(annotation) =
            self.review().and_then(|review| review.annotations.iter().find(|a| a.id == id))
        else {
            return Effect::None;
        };
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
        });
        self.editing_id = Some(id);
        Effect::SaveDraft
    }

    fn editor_key(&mut self, key: KeyEvent) -> Effect {
        let Some(editor) = &mut self.editor else { return Effect::None };
        match editor.handle_key(key, &self.config.labels) {
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
                self.commit_editor();
                Effect::SaveDraft
            }
        }
    }

    /// Put the editor into the review: a new annotation, or the change of an old one.
    fn commit_editor(&mut self) {
        let (Some(Editor { kind, range, text, label }), Some(reply)) =
            (self.editor.take(), self.shown_id.clone())
        else {
            return;
        };
        let review = self.reviews.entry(reply.clone()).or_default();
        let existing = self.editing_id.take().filter(|id| review.annotations.iter().any(|a| a.id == *id));
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
            Some(index) => self.set_cursor_line(index),
            None => self.status = Some("no match".to_owned()),
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

/// A character of a word: a letter, a digit, or an underscore.
fn is_word(c: char) -> bool {
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
