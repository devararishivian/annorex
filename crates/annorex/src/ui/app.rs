//! The state of the viewer and its keys.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::agent::Agent;
use crate::doc::Doc;
use crate::replies::{Candidate, Reply};
use crate::safety::Safety;
use crate::wrap::WrapCache;

/// The agent in the target pane and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentState {
    pub agent: Agent,
    pub safety: Safety,
}

impl AgentState {
    /// For example `claude (idle)`, `pi (working…)`, `opencode (waiting for permission)`.
    pub fn header_text(&self) -> String {
        let name = self.agent.host().label();
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
}

/// Where the keys go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Doc,
    Panel,
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
}

impl App {
    pub fn new(state: AgentState, screen: Screen, replies: Vec<Reply>) -> Self {
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
        self.doc
            .lines
            .iter()
            .enumerate()
            .rev()
            .find(|(_, line)| !line.is_blank() && line.source.start <= self.cursor)
            .map_or(0, |(index, _)| index)
    }

    /// Put `doc_line` at the top of the view and the cursor on it.
    pub fn jump_to_line(&mut self, doc_line: usize) {
        if let Some(line) = self.doc.lines.get(doc_line) {
            self.top = doc_line;
            self.cursor = line.source.start;
        }
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
            }
            None => self.show_reply(0),
        }
    }

    pub fn set_state(&mut self, state: AgentState) {
        self.state = state;
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
        self.selected = index;
        self.panel_index = index;
        self.doc = Doc::parse(&reply.text);
        self.cache.clear();
        self.top = 0;
        self.cursor = 0;
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

    fn doc_key(&mut self, key: KeyEvent) -> Effect {
        let half = (self.page / 2).max(1);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') => return Effect::Quit,
            KeyCode::Char('r') => return Effect::Reload,
            KeyCode::Char('z') => return Effect::ToggleZoom,
            KeyCode::Char('j') | KeyCode::Down => self.move_cursor(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(-1),
            KeyCode::Char('d') if ctrl => self.move_cursor(signed(half)),
            KeyCode::Char('u') if ctrl => self.move_cursor(-signed(half)),
            KeyCode::PageDown => self.move_cursor(signed(self.page.max(1))),
            KeyCode::PageUp => self.move_cursor(-signed(self.page.max(1))),
            KeyCode::Char('g') | KeyCode::Home => self.set_cursor_line(0),
            KeyCode::Char('G') | KeyCode::End => self.set_cursor_line(self.doc.lines.len().saturating_sub(1)),
            KeyCode::Char('/') => self.find_input = Some(String::new()),
            KeyCode::Char('n') => self.find_next(true, false),
            KeyCode::Char('N') => self.find_next(false, false),
            KeyCode::Char('p') => {
                self.panel_open = !self.panel_open;
                if self.panel_open {
                    self.focus = Focus::Panel;
                    self.panel_index = self.selected;
                }
            }
            KeyCode::Tab if self.panel_open => self.focus = Focus::Panel,
            _ => {}
        }
        Effect::None
    }

    fn panel_key(&mut self, key: KeyEvent) -> Effect {
        match key.code {
            KeyCode::Char('q') => return Effect::Quit,
            KeyCode::Char('r') => return Effect::Reload,
            KeyCode::Char('z') => return Effect::ToggleZoom,
            KeyCode::Char('j') | KeyCode::Down => {
                self.panel_index = (self.panel_index + 1).min(self.replies.len().saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => self.panel_index = self.panel_index.saturating_sub(1),
            KeyCode::Enter => {
                self.show_reply(self.panel_index);
                self.focus = Focus::Doc;
                if self.overlay {
                    self.panel_open = false;
                }
            }
            KeyCode::Char('p') => {
                self.panel_open = false;
                self.focus = Focus::Doc;
            }
            KeyCode::Esc | KeyCode::Tab => {
                self.focus = Focus::Doc;
                if self.overlay {
                    self.panel_open = false;
                }
            }
            _ => {}
        }
        Effect::None
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
            KeyCode::Char(c) => input.push(c),
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
        if let Some(line) = self.doc.lines.get(index) {
            self.cursor = line.source.start;
        }
    }

    /// Move the cursor by `delta` text lines. Blank lines do not count.
    fn move_cursor(&mut self, delta: isize) {
        let text: Vec<usize> =
            self.doc.lines.iter().enumerate().filter(|(_, line)| !line.is_blank()).map(|(i, _)| i).collect();
        let Some(position) = text.iter().position(|&index| index == self.cursor_line()) else { return };
        let target = position.saturating_add_signed(delta).min(text.len().saturating_sub(1));
        if let Some(&index) = text.get(target) {
            self.set_cursor_line(index);
        }
    }
}

fn signed(value: usize) -> isize {
    isize::try_from(value).unwrap_or(isize::MAX)
}

/// Case-insensitive search, character by character.
pub(crate) fn contains_ignore_case(text: &str, query: &str) -> bool {
    !match_ranges(text, query).is_empty()
}

/// The character ranges of each match of `query` in `text`, without overlap.
pub(crate) fn match_ranges(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
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
