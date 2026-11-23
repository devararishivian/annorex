# Changelog

All notable changes to Annorex are in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). This project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-11-23

### Added

- OhMyPi (`omp`) is a supported agent, on a best-effort basis. Annorex finds OhMyPi in a pane, shows its replies, and
  lists the files that it changed. Annorex reads the default profile only.
- The README has a "Supported agents" table with the minimum version of each agent and the version that Annorex was
  checked with.
- The card rail marks one active note with a thick border and the sign `▸`. The cards that are not at the focus are dim.
  The margin dot of the active note is `◆`.
- The list of replies shows the selected reply while you move in it. `esc` and `p` show the earlier reply again.

### Changed

- When several notes share a place, the edit and remove keys move the focus to the card rail. The numbered pick is gone.
  The footer says `N notes here · ↑↓ choose · KEY remove · esc back`.
- The question to remove a note shows the kind and the text of the note: `Remove this comment? "TEXT" · y remove · n
  keep`. The other footer questions are clearer too.
- The agent name of Codex is "Codex CLI" everywhere.

### Fixed

- Annorex lists the files that an agent changed when the agent gives a path through a link to the real folder, for
  example `/tmp` instead of `/private/tmp`.

## [0.4.0] - 2026-11-23

### Added

- `annorex rex install` adds the setup block to `init.lua`. It creates the file when it is missing. It appends the
  block when `init.lua` does not load Annorex. It leaves out a `rex.bind` line for a key that `init.lua` already binds.
  It checks the new file with `rex config check` and does not write when the check fails. Then it runs
  `rex config reload`. The option `--no-init` writes the module only.
- `annorex update` runs `annorex rex install` after a successful upgrade.

### Changed

- When the focused pane runs no supported agent, the action always asks which agent pane to open, also when the
  session has only one. The title of the question gives the reason, for example "omp is not supported".
- Annorex runs once for each Rex tab, not once for each session. A second tab opens its own Annorex. The notice for
  another agent in the same tab now says that Annorex is already open in this tab.
- OpenCode 2.3.2 is the first supported OpenCode version.

### Fixed

- Annorex shows the replies and the changed files of OpenCode 2.3.0 and later. It reads them from the table
  `session_part`.
- Annorex finds the Claude Code project folder for paths over 200 characters and for characters outside the basic plane.

## [0.3.0] - 2026-11-21

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
- The commands `annorex probe`, `annorex deliver`, `annorex last`, `annorex files`, `annorex rex install`, and
  `annorex rex open last|files`.
- Installers for Homebrew and for the shell, and a Lua module for Rex.

[Unreleased]: https://github.com/devararishivian/annorex/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/devararishivian/annorex/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/devararishivian/annorex/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/devararishivian/annorex/compare/v0.2.3...v0.3.0
[0.2.3]: https://github.com/devararishivian/annorex/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/devararishivian/annorex/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/devararishivian/annorex/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/devararishivian/annorex/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/devararishivian/annorex/releases/tag/v0.1.0
