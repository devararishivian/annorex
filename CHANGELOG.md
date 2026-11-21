# Changelog

All notable changes to Annorex are in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). This project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Codex CLI is a supported agent. Annorex finds Codex in a pane, shows the final answers of Codex, and lists the files
  that Codex changed. Codex needs no extension and no install step.
- Agent picker. If the focused pane is not an agent, the Rex actions find the agent panes of the session. With one agent
  pane, the action opens it. With two or more, the command palette asks which one to open.
- The command `annorex rex agents` prints the agent panes of a Rex session as JSON.

### Changed

- `annorex rex open` exits with code 5 when the target pane runs no agent. Before, it exited with code 3.

### Fixed

- A long status text no longer pushes `? help` and `q quit` out of the footer.
- `ctrl+esc` no longer closes the numbered list that picks one note.

## [0.2.3] - 2026-11-18

### Fixed

- The mouse wheel and `G` show the last rows of a reply that ends with a tall row, such as a table.
- A long list item or quote wraps under its text, not under the marker.
- `ctrl+d` and `ctrl+y` no longer answer a question by accident.
- Each `rex` call has a timeout of 8 seconds. `rex` can then report a connection problem with a clear message.

### Changed

- The file tree and the fuzzy find list the files that `.gitignore` hides. Press `.` in the tree to show or hide the
  files whose names start with a dot. Build and version-control folders never show.
- In a narrow pane, the footer hints use up to three rows. Below 16 rows, the footer has one row.

## [0.2.2] - 2026-11-16

### Added

- The note box has a caret. The arrow keys, `home`, `end`, `ctrl+a`, and `ctrl+e` move it. `ctrl+left` and
  `ctrl+right` move it by word. `ctrl+w`, `ctrl+u`, and `ctrl+k` delete text. A click in the text moves the caret.

### Changed

- `P` asks before it pastes the notes into the agent prompt without Enter. After the paste, Annorex closes.
- `S`, `P`, and `Y` work in the document, the file tree, and the list of notes. They do nothing while you write a note.
- The footer shows `S send`, `P paste`, and `Y copy`. In a narrow pane, the note keys and the send keys come first.

## [0.2.1] - 2026-11-15

### Fixed

- A note quotes the whole text of a list item, also when the item starts with bold text or holds a heading or a code
  block.
- A list with blank lines between its items keeps them in the viewer.
- Inline math such as `$\rightarrow$` and `$O(1)$` shows as `→` and `O(1)`. The note still quotes the original text.
- The card of a note on the last block shows when you scroll to the end.
- The layer header reads `Annorex review · block:<id>`. Layers from version 0.2.0 still work.

## [0.2.0] - 2026-11-14

### Added

- Update notice. The viewer shows one line when a newer release exists. Annorex checks GitHub at most once each 24
  hours, in the background.
- The command `annorex update` installs the newest release. `annorex update --check` only checks.
- The Lua option `update_check = false` and the variable `ANNOREX_NO_UPDATE_CHECK=1` turn off the check.
- After an upgrade, Annorex writes its own `annorex.lua` the next time it opens, and Rex loads it again.

### Fixed

- Annorex finds the agent when a login shell started it with `exec`.
- `h` and `l` move one character at a time inside inline code.
- The selection color stays visible over a find match.

## [0.1.0] - 2026-11-14

### Added

- The first release. Annorex opens over the full window of a Claude Code, Pi, or OpenCode pane in Rex on macOS.
- A viewer for the replies of the agent and for the files of its work directory, with a file tree and a fuzzy find.
- Notes on text: looks good, comment, remove, and replace. A mouse and a keyboard both work.
- `S` sends all notes to the agent in one message. `P` pastes the notes without Enter. `Y` copies them.
- Drafts. Annorex saves the notes to disk and deletes them only after a send that finished.
- The commands `annorex probe`, `annorex deliver`, `annorex last`, `annorex files`, and `annorex rex install`.
- Installers for Homebrew and for the shell, and a Lua module for Rex.

[Unreleased]: https://github.com/devararishivian/annorex/compare/v0.2.3...HEAD
[0.2.3]: https://github.com/devararishivian/annorex/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/devararishivian/annorex/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/devararishivian/annorex/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/devararishivian/annorex/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/devararishivian/annorex/releases/tag/v0.1.0
