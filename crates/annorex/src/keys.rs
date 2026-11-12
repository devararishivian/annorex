//! The keys of the viewer: actions, default keys, and the overrides of the config file.
//!
//! The keys for moving between panels, confirming, and cancelling are fixed. The config file
//! cannot bind them, because the viewer must always stay usable.

use std::collections::{BTreeMap, HashMap};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// One thing that a key can do in the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Left,
    Right,
    Down,
    Up,
    WordNext,
    WordPrev,
    LineStart,
    LineEnd,
    HalfPageDown,
    HalfPageUp,
    Top,
    Bottom,
    Select,
    SelectLines,
    Comment,
    LooksGood,
    Delete,
    Replace,
    Label,
    Annotations,
    Replies,
    Find,
    FindNext,
    FindPrev,
    Send,
    SendWithNote,
    Paste,
    Copy,
    Discard,
    Edit,
    Remove,
    Reload,
    PrevFile,
    NextFile,
    CloseFile,
    Tree,
    FindFile,
    Help,
    Quit,
}

impl Action {
    /// Every action, in the order of the documentation.
    pub const ALL: [Action; 39] = [
        Action::Left,
        Action::Right,
        Action::Down,
        Action::Up,
        Action::WordNext,
        Action::WordPrev,
        Action::LineStart,
        Action::LineEnd,
        Action::HalfPageDown,
        Action::HalfPageUp,
        Action::Top,
        Action::Bottom,
        Action::Select,
        Action::SelectLines,
        Action::Comment,
        Action::LooksGood,
        Action::Delete,
        Action::Replace,
        Action::Label,
        Action::Annotations,
        Action::Replies,
        Action::Find,
        Action::FindNext,
        Action::FindPrev,
        Action::Send,
        Action::SendWithNote,
        Action::Paste,
        Action::Copy,
        Action::Discard,
        Action::Edit,
        Action::Remove,
        Action::Reload,
        Action::PrevFile,
        Action::NextFile,
        Action::CloseFile,
        Action::Tree,
        Action::FindFile,
        Action::Help,
        Action::Quit,
    ];

    /// The name of the action in the config file.
    pub const fn name(self) -> &'static str {
        match self {
            Action::Left => "left",
            Action::Right => "right",
            Action::Down => "down",
            Action::Up => "up",
            Action::WordNext => "word_next",
            Action::WordPrev => "word_prev",
            Action::LineStart => "line_start",
            Action::LineEnd => "line_end",
            Action::HalfPageDown => "half_page_down",
            Action::HalfPageUp => "half_page_up",
            Action::Top => "top",
            Action::Bottom => "bottom",
            Action::Select => "select",
            Action::SelectLines => "select_lines",
            Action::Comment => "comment",
            Action::LooksGood => "looks_good",
            Action::Delete => "delete",
            Action::Replace => "replace",
            Action::Label => "label",
            Action::Annotations => "annotations",
            Action::Replies => "replies",
            Action::Find => "find",
            Action::FindNext => "find_next",
            Action::FindPrev => "find_prev",
            Action::Send => "send",
            Action::SendWithNote => "send_with_note",
            Action::Paste => "paste",
            Action::Copy => "copy",
            Action::Discard => "discard",
            Action::Edit => "edit",
            Action::Remove => "remove",
            Action::Reload => "reload",
            Action::PrevFile => "prev_file",
            Action::NextFile => "next_file",
            Action::CloseFile => "close_file",
            Action::Tree => "tree",
            Action::FindFile => "find_file",
            Action::Help => "help",
            Action::Quit => "quit",
        }
    }

    /// The default key of the action, in the text form of the config file.
    pub const fn default_key(self) -> &'static str {
        match self {
            Action::Left => "h",
            Action::Right => "l",
            Action::Down => "j",
            Action::Up => "k",
            Action::WordNext => "w",
            Action::WordPrev => "b",
            Action::LineStart => "0",
            Action::LineEnd => "$",
            Action::HalfPageDown => "ctrl+d",
            Action::HalfPageUp => "ctrl+u",
            Action::Top => "g",
            Action::Bottom => "G",
            Action::Select => "v",
            Action::SelectLines => "V",
            Action::Comment => "c",
            Action::LooksGood => "o",
            Action::Delete => "d",
            Action::Replace => "s",
            Action::Label => "L",
            Action::Annotations => "a",
            Action::Replies => "p",
            Action::Find => "/",
            Action::FindNext => "n",
            Action::FindPrev => "N",
            Action::Send => "S",
            Action::SendWithNote => "M",
            Action::Paste => "P",
            Action::Copy => "Y",
            Action::Discard => "X",
            Action::Edit => "e",
            Action::Remove => "x",
            Action::Reload => "r",
            Action::PrevFile => "[",
            Action::NextFile => "]",
            Action::CloseFile => "W",
            Action::Tree => "T",
            Action::FindFile => "ctrl+p",
            Action::Help => "?",
            Action::Quit => "q",
        }
    }

    /// What the action does, for the help screen.
    pub const fn help(self) -> &'static str {
        match self {
            Action::Left => "move left",
            Action::Right => "move right",
            Action::Down => "move down",
            Action::Up => "move up",
            Action::WordNext => "next word",
            Action::WordPrev => "previous word",
            Action::LineStart => "start of the line",
            Action::LineEnd => "end of the line",
            Action::HalfPageDown => "half a page down",
            Action::HalfPageUp => "half a page up",
            Action::Top => "go to the start",
            Action::Bottom => "go to the end",
            Action::Select => "select characters",
            Action::SelectLines => "select lines",
            Action::Comment => "comment",
            Action::LooksGood => "looks good",
            Action::Delete => "mark for removal",
            Action::Replace => "replace",
            Action::Label => "edit the note at the cursor",
            Action::Annotations => "list of notes",
            Action::Replies => "list of documents",
            Action::Find => "find text",
            Action::FindNext => "next match",
            Action::FindPrev => "previous match",
            Action::Send => "send the notes",
            Action::SendWithNote => "general note, then send",
            Action::Paste => "paste without Enter",
            Action::Copy => "copy the notes",
            Action::Discard => "discard the notes",
            Action::Edit => "edit a note",
            Action::Remove => "delete a note",
            Action::Reload => "read again",
            Action::PrevFile => "previous file",
            Action::NextFile => "next file",
            Action::CloseFile => "close the file",
            Action::Tree => "file tree",
            Action::FindFile => "find a file",
            Action::Help => "this help",
            Action::Quit => "close the viewer",
        }
    }

    fn from_name(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|action| action.name() == name)
    }
}

/// One key with its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    /// The chord of a key event. SHIFT is dropped for characters, because the character is
    /// already upper case.
    fn of_event(event: KeyEvent) -> Self {
        let mut modifiers = event.modifiers;
        if matches!(event.code, KeyCode::Char(_)) {
            modifiers.remove(KeyModifiers::SHIFT);
        }
        Self { code: event.code, modifiers }
    }

    /// The text of the key, as `parse_key` reads it.
    pub fn text(self) -> String {
        let mut out = String::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            out.push_str("ctrl+");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            out.push_str("alt+");
        }
        match self.code {
            KeyCode::Char(' ') => out.push_str("space"),
            KeyCode::Char(c) => out.push(c),
            KeyCode::Enter => out.push_str("enter"),
            KeyCode::Esc => out.push_str("esc"),
            KeyCode::Tab => out.push_str("tab"),
            KeyCode::Backspace => out.push_str("backspace"),
            KeyCode::Up => out.push_str("up"),
            KeyCode::Down => out.push_str("down"),
            KeyCode::Left => out.push_str("left"),
            KeyCode::Right => out.push_str("right"),
            KeyCode::PageUp => out.push_str("pageup"),
            KeyCode::PageDown => out.push_str("pagedown"),
            KeyCode::Home => out.push_str("home"),
            KeyCode::End => out.push_str("end"),
            other => out.push_str(&format!("{other:?}").to_lowercase()),
        }
        out
    }

    fn is_fixed(self) -> bool {
        match self.code {
            KeyCode::Enter
            | KeyCode::Esc
            | KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End => true,
            KeyCode::Char('c') => self.modifiers.contains(KeyModifiers::CONTROL),
            _ => false,
        }
    }
}

/// Parse the text of a key: one character, or a name, with optional `ctrl+` and `alt+` prefixes.
///
/// The names are `enter`, `esc`, `tab`, `space`, `backspace`, `up`, `down`, `left`, `right`,
/// `pageup`, `pagedown`, `home`, and `end`.
pub fn parse_key(text: &str) -> Result<KeyChord, String> {
    let bad = || format!("`{text}` is not a key");
    let (prefix, name) = if text == "+" {
        (None, "+")
    } else if let Some(prefix) = text.strip_suffix("++") {
        (Some(prefix), "+")
    } else {
        match text.rsplit_once('+') {
            Some((prefix, name)) => (Some(prefix), name),
            None => (None, text),
        }
    };
    let mut modifiers = KeyModifiers::NONE;
    for part in prefix.into_iter().flat_map(|prefix| prefix.split('+')) {
        match part {
            "ctrl" => modifiers |= KeyModifiers::CONTROL,
            "alt" => modifiers |= KeyModifiers::ALT,
            _ => return Err(bad()),
        }
    }
    let code = match name {
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        _ => {
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => return Err(bad()),
            }
        }
    };
    Ok(KeyChord { code, modifiers })
}

/// Actions whose default key came after the first release of the `[keys]` table.
const LATE: [Action; 1] = [Action::Help];

/// The keys of the viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMap {
    map: HashMap<KeyChord, Action>,
}

impl Default for KeyMap {
    fn default() -> Self {
        Self::with_overrides(&BTreeMap::new()).unwrap_or(Self { map: HashMap::new() })
    }
}

impl KeyMap {
    /// The default keys with the overrides of the `[keys]` table applied.
    ///
    /// The error names the key text and the actions that it concerns.
    pub fn with_overrides(overrides: &BTreeMap<String, String>) -> Result<Self, String> {
        for name in overrides.keys() {
            if Action::from_name(name).is_none() {
                return Err(format!("unknown action `{name}` in [keys]"));
            }
        }
        let mut map: HashMap<KeyChord, Action> = HashMap::new();
        // The overrides first, then the defaults. A late default (a key added after configs
        // existed) gives way to an override, so it never breaks an old config. Other collisions
        // are errors.
        let overridden = Action::ALL.into_iter().filter(|action| overrides.contains_key(action.name()));
        let defaults = Action::ALL.into_iter().filter(|action| !overrides.contains_key(action.name()));
        for action in overridden.chain(defaults) {
            let text = overrides.get(action.name()).map_or(action.default_key(), String::as_str);
            let chord = parse_key(text).map_err(|error| format!("{error} (action `{}`)", action.name()))?;
            if chord.is_fixed() {
                return Err(format!(
                    "`{text}` is a fixed key and cannot be bound (action `{}`)",
                    action.name()
                ));
            }
            if LATE.contains(&action) && !overrides.contains_key(action.name()) && map.contains_key(&chord) {
                continue;
            }
            if let Some(other) = map.insert(chord, action) {
                return Err(format!(
                    "the key `{text}` is bound to both `{}` and `{}`",
                    other.name(),
                    action.name()
                ));
            }
        }
        Ok(Self { map })
    }

    /// The key of an action as text, for the hints. An action without a key shows `unbound`.
    pub fn label(&self, action: Action) -> String {
        self.map
            .iter()
            .find(|(_, bound)| **bound == action)
            .map_or_else(|| "unbound".to_owned(), |(chord, _)| chord.text())
    }

    /// The key of an action, for a click on its hint. `None` when the action has no key.
    pub fn chord(&self, action: Action) -> Option<KeyChord> {
        self.map.iter().find(|(_, bound)| **bound == action).map(|(chord, _)| *chord)
    }

    /// The action of a key event, if the key is bound.
    pub fn action(&self, event: KeyEvent) -> Option<Action> {
        self.map.get(&KeyChord::of_event(event)).copied()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::collections::BTreeMap;

    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{Action, KeyChord, KeyMap, parse_key};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn overrides(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
    }

    #[test]
    fn default_keys_map_the_actions() {
        let map = KeyMap::default();
        let none = KeyModifiers::NONE;
        assert_eq!(map.action(key(KeyCode::Char('c'), none)), Some(Action::Comment));
        assert_eq!(map.action(key(KeyCode::Char('$'), none)), Some(Action::LineEnd));
        assert_eq!(map.action(key(KeyCode::Char('L'), none)), Some(Action::Label));
        assert_eq!(map.action(key(KeyCode::Char('d'), KeyModifiers::CONTROL)), Some(Action::HalfPageDown));
        assert_eq!(map.action(key(KeyCode::Char('d'), none)), Some(Action::Delete));
        assert_eq!(map.action(key(KeyCode::Char('q'), none)), Some(Action::Quit));
        assert_eq!(map.action(key(KeyCode::Char('?'), none)), Some(Action::Help));
    }

    #[test]
    fn every_action_has_its_own_default_key() {
        let map = KeyMap::default();
        for action in Action::ALL {
            let chord = parse_key(action.default_key()).expect("default key text");
            assert_eq!(map.action(key(chord.code, chord.modifiers)), Some(action), "{}", action.name());
        }
    }

    #[test]
    fn shifted_letter_matches() {
        let map = KeyMap::default();
        assert_eq!(map.action(key(KeyCode::Char('G'), KeyModifiers::SHIFT)), Some(Action::Bottom));
        assert_eq!(map.action(key(KeyCode::Char('G'), KeyModifiers::NONE)), Some(Action::Bottom));
    }

    #[test]
    fn key_text_parses() {
        let chord = |code, modifiers| Ok(KeyChord { code, modifiers });
        assert_eq!(parse_key("ctrl+d"), chord(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(parse_key("$"), chord(KeyCode::Char('$'), KeyModifiers::NONE));
        assert_eq!(parse_key("G"), chord(KeyCode::Char('G'), KeyModifiers::NONE));
        assert_eq!(parse_key("enter"), chord(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(parse_key("alt+x"), chord(KeyCode::Char('x'), KeyModifiers::ALT));
        assert_eq!(parse_key("space"), chord(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(parse_key("pagedown"), chord(KeyCode::PageDown, KeyModifiers::NONE));
    }

    #[test]
    fn bad_key_text_names_the_text() {
        for text in ["", "ctrl+", "banana", "ctrl+ab"] {
            let error = parse_key(text).expect_err(text);
            assert!(error.contains(&format!("`{text}`")), "{error}");
        }
    }

    #[test]
    fn an_override_replaces_the_default() {
        let map = KeyMap::with_overrides(&overrides(&[("comment", "y")])).expect("map");
        let none = KeyModifiers::NONE;
        assert_eq!(map.action(key(KeyCode::Char('y'), none)), Some(Action::Comment));
        assert_eq!(map.action(key(KeyCode::Char('c'), none)), None);
    }

    #[test]
    fn same_key_twice_names_both_actions() {
        let error = KeyMap::with_overrides(&overrides(&[("comment", "d")])).expect_err("same key");
        assert!(
            error.contains("`d`") && error.contains("`comment`") && error.contains("`delete`"),
            "{error}"
        );
    }

    #[test]
    fn unknown_action_names_the_key() {
        let error = KeyMap::with_overrides(&overrides(&[("fly", "f")])).expect_err("unknown action");
        assert!(error.contains("`fly`"), "{error}");
    }

    #[test]
    fn fixed_keys_cannot_be_bound() {
        for text in ["enter", "esc", "tab", "up", "pagedown", "home", "ctrl+c"] {
            let error = KeyMap::with_overrides(&overrides(&[("comment", text)])).expect_err(text);
            assert!(error.contains("fixed"), "{text}: {error}");
        }
    }

    #[test]
    fn an_override_can_take_the_default_key_of_another_action() {
        // `?` had no action before `help` came. A config that gave `?` to `find` must still load.
        let overrides = BTreeMap::from([("find".to_owned(), "?".to_owned())]);
        let map = KeyMap::with_overrides(&overrides).expect("loads");
        assert_eq!(map.action(key(KeyCode::Char('?'), KeyModifiers::NONE)), Some(Action::Find));
        assert_eq!(map.label(Action::Help), "unbound");
    }

    #[test]
    fn two_overrides_on_one_key_are_an_error() {
        let overrides =
            BTreeMap::from([("find".to_owned(), "?".to_owned()), ("help".to_owned(), "?".to_owned())]);
        assert!(KeyMap::with_overrides(&overrides).is_err());
    }
}
