# Annorex guide

This guide is the full reference for Annorex. It lists all keys, layouts, commands, and configuration. For a short
start, read the [README](../README.md). For the design, read [ARCHITECTURE.md](ARCHITECTURE.md).

## Contents

- [What Annorex does](#what-annorex-does)
- [Install](#install)
- [Update](#update)
- [Lua options](#lua-options)
- [Review the last reply](#review-the-last-reply)
- [Help screen](#help-screen)
- [Annotate and send](#annotate-and-send)
- [Review files](#review-files)
- [Drafts](#drafts)
- [The configuration file](#the-configuration-file)
- [Theme](#theme)
- [Use the commands](#use-the-commands)
- [Troubleshooting](#troubleshooting)
- [Known limits](#known-limits)
- [How Annorex differs from plannotator-tui](#how-annorex-differs-from-plannotator-tui)
- [How Annorex connects to Rex](#how-annorex-connects-to-rex)

## What Annorex does

- One viewer for replies and files. A Rex action opens Annorex in a floating layer over the full window of the agent.
  Annorex lets you choose a reply, and shows it as formatted Markdown. You can select one of the 25 newest replies.
  The layout follows the size of the window. A second Rex action starts the same viewer on the files of the work
  directory of the agent. A tree and a fuzzy find help you to open files. You can open many files and annotate each
  one.
- Notes. Select text in a reply or in a file. Add a comment, mark the text as good, mark the text for removal, or give
  a replacement.
- Send, paste, and copy. Annorex exports your notes as Markdown. It sends them to the agent, pastes them without
  Enter, or copies them. A send gives only the notes that Annorex did not send before. The notes of all replies and
  open files go in one message.
- Drafts. Annorex saves your notes to disk after each change. It deletes notes only after a send that it finished.
- New reply banner. When the agent answers while Annorex is open, a banner says `The agent has a new reply. Press r
  to read it.` Annorex reads the transcript every 2 seconds.
- Change banner. When the agent changes an open file, a banner shows. Press `r` to read the file again. Your notes
  move to their text.
- Agent picker. When the focused pane is not an agent, Annorex finds the agent panes of the session and asks which one
  to open. See [The agent picker](#the-agent-picker).
- `annorex probe` shows the agent in a pane, its state, and whether Annorex can send to it.
- `annorex deliver` sends text from standard input to the agent in a pane. It removes escape characters and control
  characters, pastes the text as one bracketed paste, and presses Enter.

The tests show that a multi-line Markdown message arrives intact in Claude Code, Pi, and OpenCode, also when the agent
is busy.

## Install

You need these items:

- Rex on macOS (Apple silicon or Intel).
- Claude Code, Pi 1.5.0 or later, OpenCode 2.3.2 or later, or Codex CLI.
- Rust 1.99 or later, only to build from source.

Each release has prebuilt binaries for Apple silicon and Intel. Install the binary with Homebrew:

```sh
brew install devararishivian/tap/annorex
```

Homebrew puts the binary in its own `bin` directory: `/opt/homebrew/bin` on Apple silicon, `/usr/local/bin` on
Intel. The fully qualified name `devararishivian/tap/annorex` trusts only the Annorex formula. To trust the whole tap,
run `brew trust --tap devararishivian/tap`. Or use the shell installer of the release:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/devararishivian/annorex/releases/latest/download/annorex-installer.sh | sh
```

The shell installer puts the binary in `$CARGO_HOME/bin`, else in `~/.cargo/bin`. Set `ANNOREX_INSTALL_DIR` to use a
different directory. The binary then goes in the `bin` directory below it.

To build from source, install the binary from the repository on GitHub:

```sh
cargo install --git https://github.com/devararishivian/annorex --locked annorex
```

Or install it from a clone of the repository. Run this command in the root directory of the clone:

```sh
cargo install --path crates/annorex --locked
```

`cargo` puts the binary in `$CARGO_HOME/bin`, else in `~/.cargo/bin`. After each install method, write the Lua
module:

```sh
annorex rex install
```

`annorex rex install` writes the Lua module `annorex.lua` beside the `init.lua` of Rex. Then it adds the setup block to
`init.lua`. It finds the directory of Rex in this order: the directory of `$REX_CONFIG`, then `$XDG_CONFIG_HOME/rex`,
then `~/.config/rex`. Use `--dir DIR` to write to a different directory. The command writes `DIR/init.lua`. The reload
still makes the running Rex server read its own configuration file, not `DIR/init.lua`.

The command replaces only a module that an earlier `annorex rex install` wrote. If a different `annorex.lua` exists,
the command refuses with exit code 3. Remove or rename that file first.

This is the setup block:

```lua
-- Added by `annorex rex install`. Remove these lines to remove Annorex.
require("annorex").setup{}
rex.bind("cmd+shift+a", "annorex_last")
rex.bind("cmd+shift+f", "annorex_files")
```

The command acts on `init.lua` in this way:

- If `init.lua` is missing, the command creates the file and the folder.
- If `init.lua` does not load Annorex, the command appends the block.
- If `init.lua` already loads Annorex, the command does not change `init.lua`. This holds for each form of `require`,
  also a commented line or `pcall(require, "annorex")`. The command still checks that `init.lua` with
  `rex config check`. If the check fails, it prints the Rex message and exits with status 1. Otherwise it reloads Rex.
- If `init.lua` already binds a key of the block, the command leaves out that `rex.bind` line and names the key. Add
  the line yourself with a free key.
- If `init.lua` is a symbolic link, the link stays and the target gets the lines.

Before it writes, the command runs `rex config check` on the new file. If the check fails, the command does not write
`init.lua`. The exit code is 1, and the command prints the lines to add by hand. Without `rex` on the path, the command
writes without the check and skips the reload.

After it writes, the command runs `rex config reload`. If the reload fails, the command prints
`Could not reload Rex. Run: rex config reload`. The exit code stays 0.

To write the module only, run `annorex rex install --no-init`. The command prints the lines to add. You must add them
and run `rex config reload` yourself.

To undo the change, delete the block from the marked comment to the last `rex.bind` line. Then run
`rex config reload`.

The command prints the path of the binary that Rex starts, after `Rex starts:`. If the path is in a cargo build
directory (`target/debug` or `target/release`), the command warns you. That path changes with each build. Run
`annorex rex install` again from the installed binary. For a Homebrew binary, the module stores the stable `bin` path
(for example `/opt/homebrew/bin/annorex`), not the versioned `Cellar` path. After an upgrade, you do not need to run
`annorex rex install` again. See [Update](#update).

Annorex always opens over the full window of the agent. The agent pane keeps its size. There is no option for the
placement. The key `placement` in `config.toml` and the option `--placement` give an error.

Only one Annorex runs in each Rex tab (window). If you start Annorex while one is open in the tab, Annorex shows the
open one. It does not open a second one, also for another agent. A second tab opens its own Annorex.

Rex ends the zoom of a pane when the layer opens. Rex does not restore the zoom when you close the viewer. If the open
Annorex is for another agent, the action shows this message:

`Annorex is already open in this tab, for {agent}. It is in front now. Press q there to close it, then open Annorex again.`

The replies of the agent and the files of its work directory are in the same Annorex. `annorex_last` and
`annorex_files` open the same viewer. They differ in the first document: the newest reply, or the files that you name
(the file tree when you name none). The file viewer shows the tree on the left and the file keys in the footer. The
last-reply viewer hides the tree until you press `T`. With two or more replies, it opens the reply list first. The
newest reply is selected. Press `enter` to open the selected reply. Press `esc` to quit Annorex. The footer shows the
keys of the document on the screen: reply keys for a reply, file keys for a file. All keys work in both viewers. One
send gives the notes on all replies and files to the agent.

## Update

From version 0.2.0, Annorex checks GitHub for a newer release. When one exists, the viewer shows one banner line:
`Annorex X is available. Run: HINT`. The hint is `brew upgrade annorex` for a Homebrew install. Otherwise it is
`annorex update`.

The check runs at most once each 24 hours, in the background. A network failure shows nothing. The only request is
`GET https://api.github.com/repos/devararishivian/annorex/releases/latest`. Annorex never replaces its own binary
unless you run `annorex update`.

Run `annorex update --check` to look for a newer release and print the result. Run `annorex update` to upgrade. The
command depends on the install channel:

| Install channel | What `annorex update` does                           |
|-----------------|------------------------------------------------------|
| Homebrew        | Runs `brew upgrade annorex`                          |
| Shell installer | Runs the latest installer of the release again       |
| `cargo install` | Prints the `cargo install` command. You must run it. |

After a successful upgrade, `annorex update` runs `<binary> rex install` and prints `Running:` and the command.

After `brew upgrade`, or an update from version 0.3.0, the next open of Annorex replaces the `annorex.lua` of Rex
with the new module. It does this only for a file that Annorex wrote. Then it asks Rex to reload the configuration.
If the reload fails, the next open shows `run: rex config reload`. Run that command.

If you use version 0.3.0 and never added the setup lines to `init.lua`, run `annorex rex install` one time.

To turn off the check, use one of these ways:

- Set `update_check = false` in `setup{}`. Use this way for the Rex key actions. See [Lua options](#lua-options).
- Set the environment variable `ANNOREX_NO_UPDATE_CHECK=1`. The variable works when you start Annorex from a shell. This
  includes `annorex rex open` from a shell.

The source formula in the repository, `packaging/homebrew/annorex.rb`, builds Annorex with the check turned off. It is
prepared for homebrew-core. The tap formula `devararishivian/tap/annorex` keeps the check.

## Lua options

`setup{}` takes two options. The option `bin` gives the `annorex` binary that the actions start. The option
`update_check = false` turns off the check for a newer release. The default is `true`:

```lua
require("annorex").setup{ bin = "/path/to/annorex", update_check = false }
```

A value with a `/` is a path. The file must exist. A value without a `/` is a name that Rex finds on its own `PATH`.
The module writes a warning to the Rex log for each other option: `annorex: unknown option NAME`.

Rex runs actions with a short `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), not the `PATH` of your shell. So without
`bin`, the module finds the binary itself. It uses the first file that exists:

1. The binary that ran `annorex rex install`. The command prints this path after `Rex starts:`.
2. `$CARGO_HOME/bin/annorex`, else `~/.cargo/bin/annorex`.
3. `/opt/homebrew/bin/annorex`, then `/usr/local/bin/annorex`.
4. `~/.local/bin/annorex`.

The module adds two actions. Each action runs `annorex rex open last` or `annorex rex open files` for the focused pane,
with a timeout of 25 seconds:

| Action          | Title in the command palette     |
|-----------------|----------------------------------|
| `annorex_last`  | "Annorex: review the last reply" |
| `annorex_files` | "Annorex: review files"          |

If the focused pane is not an agent, each action also runs `annorex rex agents`. See [The agent
picker](#the-agent-picker).

## Review the last reply

Focus the pane of Claude Code, Pi, OpenCode, or Codex CLI. Press the key that you bound, or run "Annorex: review the
last reply" from the command palette. The action `annorex_last` opens the viewer over the full window. If an Annorex is
already open in the tab, the action focuses it.

The header shows the agent and its state, for example `Claude Code (idle)` or `OpenCode (waiting for permission)`.
Annorex reads the state again every 2 seconds.

Annorex has two modes in the document. Block mode is the default. Text mode is for notes on a part of a block. To enter
text mode, press `v`, `V`, or a move key such as `h` or `l`, or drag the mouse over text.

### Block mode

At the start, no block has the focus, and the action bar does not show. Press `j` to focus the first block on the
screen. Press `k` to focus the last block on the screen. A click on a block also focuses it. A note key without a focus
shows `choose a block first (j/k or a click)`. Press `esc` to remove the focus. A new document starts without a focus.
After a reload, the same document keeps its focus, or keeps no focus.

With a focus, one block has it. A block is a heading, a paragraph, a list item, a whole code block, a whole table, a
whole quote, or a rule. A nested list item is its own block. In a plain text or code file, a block is a group of lines
between blank lines. The focused block has a bar in the left margin. The note keys act on it. Press `enter` to show the
action bar above it.

| Key                       | Function                                                                                                              |
|---------------------------|-----------------------------------------------------------------------------------------------------------------------|
| `j` / `k`, arrows         | Go to the next or the previous block                                                                                  |
| `g` / `G`, `home` / `end` | Go to the first or the last block. Without a focus, only scroll to the top or the end                                 |
| `ctrl+d` / `ctrl+u`       | Scroll half a page, then focus the first block in the view. Without a focus, only scroll                              |
| `PageDown` / `PageUp`     | Scroll one page, then focus the first block in the view                                                               |
| `o` / `d`                 | Note on the block: looks good or remove (saves at once, no text, no note box)                                         |
| `c` / `s`                 | Note on the block: comment or replace (opens the note box)                                                            |
| `e` / `x`                 | Edit or delete the note of the block                                                                                  |
| `enter`                   | Show the action bar for the whole block. Block mode stays. Without a focus, focus the first block on the screen first |
| `v`                       | Enter text mode and start a selection at the start of the block                                                       |
| `V`                       | Enter text mode with the lines of the block selected                                                                  |
| `h` `l` `w` `b` `0` `$`   | Enter text mode at the start of the block, then do the move                                                           |

A block that is taller than the screen scrolls first. `j` moves to the next block when the end of the block is visible.
`k` works the same way upward. If several notes share one block, a numbered list shows. Press `1` to `9` to pick one.

### Text mode

Text mode has a free cursor and a character selection.

| Key                   | Function                                                                                                                                          |
|-----------------------|---------------------------------------------------------------------------------------------------------------------------------------------------|
| `j` / `k`, arrows     | Move down or up one line                                                                                                                          |
| `h` / `l`             | Move left or right one character                                                                                                                  |
| `w` / `b`             | Move to the next or the previous word                                                                                                             |
| `0` / `$`             | Move to the start or the end of the line                                                                                                          |
| `ctrl+d` / `ctrl+u`   | Move down or up half a page                                                                                                                       |
| `PageDown` / `PageUp` | Move down or up one page                                                                                                                          |
| `g` / `G`             | Go to the start or the end                                                                                                                        |
| `v` / `V`             | Start a selection of characters or of lines. Press the key again to end the selection and show the action bar. A third press clears the selection |
| `o` / `c` / `d` / `s` | Note on the selection, or on the line of the cursor: looks good, comment, remove, or replace                                                      |
| `esc`                 | Clear the selection. Without a selection, go back to block mode. In block mode, remove the focus. With nothing selected or focused, quit          |
| `n` / `N`             | Go to the next or the previous match (in text mode)                                                                                               |

### The action bar

The action bar shows after you finish a selection:

- when a drag of the mouse ends,
- after a double click on a word,
- after `enter` on the focused block,
- after the `v` or `V` that ends a selection.

After `enter`, the selection is the whole focused block. The action bar shows on the row above the selection. When
there is no row above, it shows on the row below. It starts at the first column of the selection. Near the right edge,
it moves left to stay in the document. It has four items: `o ✓ looks good`, `c 💬 comment`, `d ✕ remove`, and
`s ⇄ replace`. Each item has the icon and the color of its note card. A click on an item does the same as its key.

The keys `o`, `c`, `d`, and `s` also work without the action bar. They act on the focused block or on the selection.
A focus move with `j` or `k` does not show the action bar. Each key in the document closes it, except `?`. The help
screen only covers it. A mouse click outside the action bar closes it. The mouse wheel does not close it. A saved note
and an open note box close it too.

### Keys in both modes

| Key           | Function                                                         |
|---------------|------------------------------------------------------------------|
| `L`           | Edit the note at the cursor, or add a comment                    |
| `a`           | Move the focus to the list of notes                              |
| `S`           | Send the notes to the agent                                      |
| `M`           | Write a general note, then send                                  |
| `P`           | Paste the notes without Enter (asks first), then close           |
| `Y`           | Copy the notes                                                   |
| `X`           | Discard the notes of the document on the screen (asks first)     |
| `/`           | Find text (the case is ignored)                                  |
| `n` / `N`     | Go to the next or the previous match                             |
| `p`           | Open or close the list of replies and open files                 |
| `tab`         | Move the focus between the tree, the document, and the card rail |
| `r`           | Read the transcript again                                        |
| `?`           | Open the help screen                                             |
| `q`, `ctrl+c` | Close the viewer                                                 |

In the list of notes and on a card, `j` and `k` move between notes, and `enter` goes to the note. `e` and `x` edit
and delete the note. `esc` returns to the document. In the list of replies, `enter` shows the selected reply.

### The note box

The note box is a floating box below the block, or above it when there is no room below. The title names the kind and
the line, for example `Comment · line 20`. An empty box shows a dim hint. It has the buttons `save` and `cancel`.

`enter` saves the note. A new line needs `shift+enter` when the terminal supports it. Otherwise, use `ctrl+j`. The
footer shows the key that works. `alt+enter` also adds a new line. `esc` cancels.

The box has a caret. The text goes in at the caret, and a paste from the terminal goes in there too. These keys move
the caret or delete text:

| Key                                                                     | Function                                       |
|-------------------------------------------------------------------------|------------------------------------------------|
| `left` / `right`                                                        | Move by one character                          |
| `up` / `down`                                                           | Move by one row on the screen                  |
| `home` / `end`, `ctrl+a` / `ctrl+e`                                     | Go to the start or the end of the line         |
| `ctrl+left` / `ctrl+right`, `alt+left` / `alt+right`, `alt+b` / `alt+f` | Move by one word                               |
| `backspace` / `delete`                                                  | Delete the character before or after the caret |
| `ctrl+w`, `alt+backspace`, `ctrl+backspace`                             | Delete the word before the caret               |
| `alt+d`, `ctrl+delete`                                                  | Delete the word after the caret                |
| `ctrl+u` / `ctrl+k`                                                     | Delete to the start or the end of the line     |

The `alt` keys need the Rex setting "Option key" to send Alt. "Automatic" sends Alt on a U.S. keyboard layout. A click
in the text moves the caret. The keys of the note box come before the keys of the viewer, and the configuration file
does not change them.

The box shows at most 8 rows of text. It follows the caret. Press `PgUp` or `PgDn`, or use the mouse wheel over the
box, to scroll. The next key goes back to the caret. A comment needs text. The notes `looks good` and
`remove` have no text, so they save at once without the box. `e` on them shows `Looks good has no text.` or
`Remove has no text.` A draft of an older version can have text on these notes. Annorex keeps that text in the export.

The configuration file can still have `labels`. Annorex reads them and does not use them. A note with a label keeps
its label in the export.

### The layout

The layout changes with the size of the window:

- 120 columns or more: the file tree on the left (when it shows), the document, and the card rail on the right. The
  tree is 28 columns wide. The rail is 34 columns wide. A note shows as a card in the rail. When the cards do not
  fit, the rail leaves out the top cards. The document takes all the room between the tree and the rail.
- 100 to 119 columns: the file tree on the left (when it shows) and the document. There is no rail. A dot in the color
  of the note marks the note in the right margin of the block.
- Less than 100 columns, or less than 16 rows: one column. The tree and the list of notes open over the document. The
  dots also show in the right margin when the document is 21 columns or more wide.
- Less than 40 columns or less than 10 rows: Annorex shows "pane too small".

The file viewer starts with the tree. With no path, it shows no document until you open a file. Press `p` to open a
reply. The last-reply viewer starts without the tree. `T` shows the tree with the focus. A second `T` hides it. Over a
narrow window, `T` opens and closes the tree. The list of replies (`p`), the file finder (`ctrl+p`), and the session
picker open in a box in the center, at all widths.

The header shows the view, the agent and its state, and the position. It adds `unsent in reply 1,3` or
`unsent in 2 files` when notes are not sent. On the right, it has the send button `[ Send N → agent ▸ ]`. With no notes,
the button is dim: `[ Send ▸ ]`. It is dim while the agent shows a dialog too. While the agent works, the button adds
`queued` for Claude Code or `steers` for Pi, OpenCode, and Codex CLI. After a send, it shows `[ Sent HH:MM ]`. The
footer shows the keys of the current mode. The note keys and `S send`, `P paste`, and `Y copy` come first. The hints
flow onto up to three rows. The footer has one row when the window has fewer than 16 rows. It also has one row while a
question, the find input, or the note box is open. Annorex leaves out the hints that do not fit, from the end. `? help`
and `q quit` stay while they fit.

Notes show in place. The note colors are the 16 ANSI colors. The text under a note keeps its color. Looks good has a
green underline. A comment has a yellow underline. Remove is red and crossed out. Replace is magenta and crossed out. A
bar in the note color shows in the left margin of each row under a note. The cyan focus bar shows on the focused block
instead.

A table that is wider than the window gets narrower columns, and its cells wrap. Each column keeps its longest word, up
to 20 columns. If even these widths do not fit, Annorex clips the lines of the table.

### The mouse

Each mouse action has a key. The help screen does not list the mouse actions.

| Where                      | Action         | Result                                                                                                                 |
|----------------------------|----------------|------------------------------------------------------------------------------------------------------------------------|
| List, document, tree, rail | Wheel          | Scroll the part under the pointer by 3 lines. The focus does not change                                                |
| Block                      | Click          | Focus the block (block mode)                                                                                           |
| Document text              | Drag           | Select characters (text mode). At the top or the bottom row, the document scrolls. The action bar shows at the release |
| Document text              | Double click   | Select the word and show the action bar                                                                                |
| Action bar item            | Click          | Add that note. Looks good and remove save at once. The others open the note box                                        |
| Card                       | Click          | Focus the note and its block. The card shows `edit` and `remove` buttons. Looks good and remove have `remove` only     |
| Header send button         | Click          | The same as `S`, with the same dialog test and messages                                                                |
| Tree row                   | Click          | Open the file, or open or close the directory                                                                          |
| Row of a list or picker    | Click          | Open that reply, session, or file                                                                                      |
| Numbered list of notes     | Click          | Choose that note                                                                                                       |
| Footer hint                | Click          | Do that action                                                                                                         |
| Note box                   | Click          | Move the caret. Click `save` or `cancel` to save or cancel. A click outside the box does nothing. The wheel scrolls it |
| Help screen                | Click or wheel | Close the help screen                                                                                                  |

A question in the footer (`y/n`) and the find input take no mouse event. There is no right click and no hover action.
Annorex has no key that copies the selected text. `Y` copies the notes with their quoted text.

Pi and OpenCode can have more than one session in the same directory. If Annorex cannot find the session of the agent
with certainty, the list shows the sessions, newest first. Select one with `enter`. If Annorex finds no replies (for
example, the agent did not answer yet), the list says why, and `r` looks again. The files still work.

### OpenCode and Claude Code storage

Annorex supports OpenCode from version 2.3.2. OpenCode 2.3.0 and later store new replies in the table `session_part`.
Annorex reads the reply text and the tool calls from that table. Older replies of OpenCode v2 keep their text in the
table `session_message`. Annorex reads that text as before. Only OpenCode v1 uses other tables.

For Claude Code, Annorex finds the project folder in `~/.claude/projects` with the rule of Claude Code. The rule covers
paths over 200 characters and characters outside the basic plane.

### Codex CLI

Codex CLI needs no extension and no install step. Rex reports the state of Codex. Annorex reads the session files of
Codex. Annorex finds Codex in a pane through the program status of Rex (`app: "codex"`). It can also use the process
name `codex`.

Codex keeps one file for each session in `~/.codex/sessions`. If the environment variable `CODEX_HOME` is set, the
folder is `$CODEX_HOME/sessions`. Annorex lists the sessions of the work directory of the agent, newest first. It skips
the sessions of subagents.

A reply is a final answer of Codex. Annorex also counts a message that has no `phase`. Annorex does not show the
progress messages that Codex writes while it works (`phase: commentary`).

### The agent picker

The actions `annorex_last` and `annorex_files` need an agent pane. If the focused pane runs no supported agent, the
command `annorex rex open` exits with code 5. Its message names the program, for example "this pane runs omp, which
Annorex does not support". The Lua module then runs `annorex rex agents --session SESSION`. This command prints the
agent panes of the session as JSON. The action then does one of these:

- If the session has no agent pane, the action fails with the message and says that no pane runs a supported agent.
- If the session has one or more agent panes, the command palette asks which one to open. It asks also for one pane.
  The title gives the reason, for example "Annorex: this pane runs omp, which Annorex does not support. Pick an agent".
  Each choice shows the agent, its work directory, and its window. The action focuses the pane that you choose and opens
  Annorex there.

Annorex never opens another agent pane without your answer.

If you cancel the question, the action closes it and shows no message.

## Help screen

Press `?` to open a help screen. It lists all keys from the live key map, so it shows your changes from `config.toml`.
Press any key to close it. The action name in the configuration file is `help`. The footer shows `? help`.

## Annotate and send

1. Move to the block with `j` and `k`. To note a part of a block, press `v` and move to select characters, or press
   `V` to select lines. Without a selection in text mode, the note uses the line of the cursor.
2. Press `o` to mark the text as good. This needs no text. Press `c` to comment, `d` to mark the text for removal, or
   `s` to replace it. Type the note. Press `enter`.
3. Press `S` or click the send button in the header. The button shows `[ Send N → agent ▸ ]`, with N the number of
   notes. After the agent got the message, Annorex deletes the notes that the send took from their drafts and closes.
   `P` asks first. After the paste, Annorex closes and keeps the notes as sent. When Enter was held, Annorex stays
   open and keeps the notes as sent. If the send fails, Annorex stays open and keeps the drafts. The footer shows the
   reason. `S`, `P`, and `Y` work in the document, the file tree, and the list of notes, but not in the note box.

The export is Markdown. Without a general note, it starts with `# Annotations on last reply`. For an older reply, the
name is `earlier reply (HH:MM)` in your local time. If you wrote a general note with `M`, the export starts with the
heading `# Note` and the note as a quote. Then the annotation sections follow. Each note has a number, the line
numbers, the quoted source text, and your note as a quote. The text before the quote names the type: `Comment on:`,
`Remove this:`, or `Looks good:`.

Annorex does not add a second identical note. The note must match in kind, range, label, and text. The footer says
`Already saved.`

A send gives only the notes that Annorex did not send before. After you change a sent note, it counts as new. `Y`
always copies all notes. If no note is new, the footer says `Nothing to send: no annotations yet.`

### How a send works

A send follows these steps. `annorex deliver` uses the same steps.

1. Annorex makes sure that a supported agent runs in the pane.
2. It makes sure that the agent process owns its Rex status record, and that the agent shows no dialog.
3. It makes sure that the pane turned on bracketed paste.
4. It reads the status again, immediately before the paste. A dialog can open while Annorex reads.
5. It pastes the notes as one bracketed paste.
6. It waits `enter_delay_ms` (300 ms by default) and reads the status a third time.
7. If the agent shows no dialog, it presses Enter.

Annorex never sends again automatically. If a step before the paste fails, Annorex writes nothing to the agent.

An idle agent starts to work on the notes. When the agent is busy, it still gets the message. Claude Code queues the
message. Pi, OpenCode, and Codex CLI use it to steer the current task.

`S` refuses to send when the agent shows a dialog, such as a permission prompt. The footer says
`Not sent: the agent shows a permission dialog. Your notes are kept.` Your notes stay in the draft.

Annorex sends and pastes only when the agent process owns its Rex status record. Otherwise it refuses. The footer says
`Not sent: the agent has not reported its state yet; wait until it is ready. Your notes are kept.` This happens while
the agent starts or shows a prompt that it does not report. Wait until the agent is ready, then press `S` again.

If the agent opens a dialog after the paste, Annorex does not press Enter. The footer says
`Pasted, but Enter was held`. The notes are in the input box of the agent. Annorex stays open and marks the notes as
sent. Answer the dialog, then press Enter in the agent pane.

When Annorex cannot send, use `P` or `Y`. `P` pastes the notes into the input box of the agent and does not press
Enter. `Y` copies the notes with an OSC 52 sequence and with `pbcopy`. If only the OSC 52 part works, the footer says
so. `P` and `Y` do not close Annorex.

### Quit

`q` quits when no note box or text input is open. In a box, `q` types a letter. `esc` steps back one level: it closes
a box, then clears the selection, then removes the focus. When nothing is selected or focused, `esc` quits too. The
footer ends with `q quit`. It shows `esc unselect` only while something is selected or focused.

`q`, `esc` with nothing selected, and `ctrl+c` with unsent notes ask what to do: send (`s`), keep the draft (`k`),
discard (`d`), or go back (`esc`).

## Review files

Focus the pane of Claude Code, Pi, OpenCode, or Codex CLI. Press the key that you bound, or run "Annorex: review files"
from the command palette. The action `annorex_files` opens the file viewer over the full window. If an Annorex is
already open in the tab, the action focuses it. Then Annorex ignores the new paths. The root is the work directory
of the agent process.

The viewer starts with the file tree. Open a file with `enter`. Annorex shows a Markdown file (`.md` or `.markdown`) as
Markdown, without line numbers. It shows all other files as plain text with line numbers. It shows the text of a file
with control characters removed.

You can open up to 50 files. Each file has its own notes and its own draft. The keys for replies work in a file too.
These keys are for files:

| Key       | Function                                                                    |
|-----------|-----------------------------------------------------------------------------|
| `T`       | Hide or show the file tree (over a narrow window: open or close it)         |
| `.`       | Show or hide hidden files (in the tree)                                     |
| `ctrl+p`  | Open the fuzzy find of files                                                |
| `p`       | Open or close the list of replies and open files (`●` marks unsent notes)   |
| `[` / `]` | Show the previous or the next document (replies first, then the open files) |
| `W`       | Close the file on the screen (asks first when it has unsent notes)          |
| `r`       | Read the file again, and look for new replies                               |

In the tree, `j` and `k` move, and `l` and `h` open and close a directory. `enter` opens a file or a directory, and
`/` starts a filter. In the fuzzy find, type to filter by path, move with the arrows or with `ctrl+n` and `ctrl+p`, and
press `enter` to open the file. A `●` marks a file with a saved draft that has unsent notes.

The tree lists files that `.gitignore`, `.git/info/exclude`, and `.ignore` would hide. Names that start with a dot show
only after you press `.` in the tree. Press `.` again to hide them. These folders never show: `.cache`, `.direnv`,
`.git`, `.hg`, `.jj`, `.svn`, `.venv`, `__pycache__`, `build`, `dist`, `node_modules`, `out`, `target`, `vendor`, and
`venv`. The fuzzy find uses the same rules. The tree does not follow links. Annorex refuses a file that is outside the
work directory, also through a link.

Annorex reads the size and the time of each open file every 2 seconds. When a file changes on disk, a banner shows
`<path> changed on disk. Press r to read it again.` The list marks the file with `≠`. Press `r`. The text is new, and
each note moves to its quoted text. If the text is gone, the note stays and the export says `text not found`. When the
agent deletes a file, the banner says `<path> was deleted. Your notes are kept.` and the text stays.

The tree puts the files that the agent changed since your last prompt at the top, under the line
`* changed by the agent: N`, and marks them with `*`. Annorex reads the transcript of the agent: the files of its
successful `Edit`, `Write`, `edit`, `write`, and `apply_patch` calls. The fuzzy find lists them first too. When the
agent changes more files, a banner says `The agent changed N more files.` until you open the tree.

The list is a hint, not the full list. A file that the agent changed with a shell command (for example `sed -i`) or
with a tool of an MCP server is not shown. Neither is a file that the agent wrote and deleted, or that is outside the
work directory. Your own send is a prompt. After it, the list holds only what the agent changes next. Raise
`changes_turns` to look back further. Annorex follows the newest session of the agent in the work directory. With two
Pi, OpenCode, or Codex panes in one directory, the list can belong to the other pane.
For Codex, Annorex reads the `FileChange` records. It does not show a change that you declined.

One send gives the general notes and the notes of all documents in one message. It has one section for each
document: `# Annotations on last reply` (or `earlier reply (HH:MM)`) for a reply, and `# Annotations on PATH` for a
file. The replies come first. The files follow in the order in which you opened them. If you press `q` with unsent
notes, the question counts the documents.

You can give files at the start. The paths are relative to the work directory:

```sh
annorex files --target block:... README.md docs/guide.md
annorex rex open files --target block:... README.md
```

A path can name a directory. Then the tree opens there. If an Annorex is already open in the tab,
`annorex rex open files` only moves the focus to it and does not open the paths. Open them in that viewer with
`ctrl+p` or `T`.

Limits: Annorex does not open a file larger than 1 MiB or a file with a NUL byte in its first 8192 bytes. The tree
shows 20,000 entries at most and says so when there are more. It reads the folders level by level from the root, so
the deepest entries are the first to go. If you open more than 50 files, Annorex asks you to close one first.

## Drafts

Annorex saves one draft for each reply and for each file in `$XDG_STATE_HOME/annorex/drafts`. The default is
`~/.local/state/annorex/drafts`. The directory has mode `0700` and the files have mode `0600`. The draft keeps the open
note box, so a restart does not lose half a note. When you close Annorex, it keeps all notes that no send took.

After the agent got the message, Annorex deletes the notes that the send took. It deletes the draft file when nothing
else is in it. The next time that you open the reply or the file, it starts clean. The sent notes with their quoted
text do not stay on disk. After a paste with `P`, or when Enter was held, the notes stay in the draft, marked as
sent. Annorex does not delete them. You remove them with `x`.

When Annorex starts, it deletes each draft that a finished send used up and each draft with no note. It keeps a draft
that only a paste or a held Enter marked. If a draft file is
unreadable, Annorex keeps it and shows a message. Annorex does not delete a file that it cannot read, except when you
discard the notes with `X`.

## The configuration file

Annorex reads `$XDG_CONFIG_HOME/annorex/config.toml`, or `~/.config/annorex/config.toml`. All values are optional.

```toml
enter_delay_ms = 300       # 50 to 5000: the wait between the paste and Enter
labels = ["nit"]           # read, but the note box does not use labels
changes_turns = 1          # 1 to 20: how many of your prompts the tree looks back for changed files
                           # (default 1)

[keys]
comment = "y"              # an action name and a key
```

A bad configuration file stops Annorex with exit code 64 and a message. An unknown key at the top level is an error
too. The viewer exits when its terminal closes.

### Key names

A key is one character, or one of these names: `enter`, `esc`, `tab`, `space`, `backspace`, `up`, `down`, `left`,
`right`, `pageup`, `pagedown`, `home`, and `end`. Put `ctrl+` or `alt+` before a key for a modifier, for example
`ctrl+d`. An upper-case letter is the letter with Shift.

The keys `enter`, `esc`, `tab`, the arrows, the page keys, `home`, `end`, and `ctrl+c` are fixed. The `[keys]` table
cannot bind them. A key that two actions use is an error. An unknown action name is an error.

### Action names

The `[keys]` table takes these action names.

| Action name      | Default key | Function                         |
|------------------|-------------|----------------------------------|
| `left`           | `h`         | Move left                        |
| `right`          | `l`         | Move right                       |
| `down`           | `j`         | Move down                        |
| `up`             | `k`         | Move up                          |
| `word_next`      | `w`         | Next word                        |
| `word_prev`      | `b`         | Previous word                    |
| `line_start`     | `0`         | Start of the line                |
| `line_end`       | `$`         | End of the line                  |
| `half_page_down` | `ctrl+d`    | Half a page down                 |
| `half_page_up`   | `ctrl+u`    | Half a page up                   |
| `top`            | `g`         | Go to the start                  |
| `bottom`         | `G`         | Go to the end                    |
| `select`         | `v`         | Select characters                |
| `select_lines`   | `V`         | Select lines                     |
| `comment`        | `c`         | Comment                          |
| `looks_good`     | `o`         | Looks good                       |
| `delete`         | `d`         | Mark for removal                 |
| `replace`        | `s`         | Replace                          |
| `label`          | `L`         | Edit the note at the cursor      |
| `annotations`    | `a`         | List of notes                    |
| `replies`        | `p`         | List of documents                |
| `find`           | `/`         | Find text                        |
| `find_next`      | `n`         | Next match                       |
| `find_prev`      | `N`         | Previous match                   |
| `send`           | `S`         | Send the notes                   |
| `send_with_note` | `M`         | General note, then send          |
| `paste`          | `P`         | Paste without Enter              |
| `copy`           | `Y`         | Copy the notes                   |
| `discard`        | `X`         | Discard the notes                |
| `edit`           | `e`         | Edit a note                      |
| `remove`         | `x`         | Delete a note                    |
| `reload`         | `r`         | Read again                       |
| `prev_file`      | `[`         | Previous file                    |
| `next_file`      | `]`         | Next file                        |
| `close_file`     | `W`         | Close the file                   |
| `tree`           | `T`         | File tree                        |
| `hidden`         | `.`         | Show or hide hidden files        |
| `find_file`      | `ctrl+p`    | Find a file                      |
| `help`           | `?`         | This help                        |
| `quit`           | `q`         | Quit (not while you type a note) |

The default key of `help` gives way to your own binding. If you bind `?` to another action, `help` has no key.

## Theme

Annorex uses the 16 ANSI colors of the terminal. It follows the Rex theme, light or dark. Rex gives programs no way to
read the theme. Notes have no background fill. They use an underline and a bar in the left margin.

## Use the commands

Run the commands in a Rex pane. Give the block ID of the agent pane as `--target`. The session is `$REX_SESSION`, or
give it with `--session`.

```sh
annorex probe --target block:...
annorex probe --target block:... --json
printf 'Fix the test in parser.rs.\n' | annorex deliver --target block:...
printf 'Fix the test in parser.rs.\n' | annorex deliver --target block:... --paste-only
annorex last --target block:...
annorex rex open last --target block:...
annorex files --target block:... [PATH...]
annorex rex open files --target block:... [PATH...]
annorex rex agents --session SESSION
annorex rex install [--dir DIR] [--no-init]
annorex update [--check]
```

`annorex probe` prints the agent, its state, a dialog when one shows, the bracketed paste mode, and whether a send is
allowed. While the agent works, it also prints what a send does now: `queued` or `steers the current task`. With
`--json`, it prints one JSON object with the fields `agent`, `state`, `kind`, `msg`, `bracketed_paste`,
`send_allowed`, and `busy_effect`.

`annorex last` runs the viewer in the current terminal. `annorex rex open last` opens the layer with the viewer, as the
action does. It prints the block ID of the viewer as JSON. If an Annorex is already open, the JSON has the field
`existing`. `annorex files` and `annorex rex open files` do the same for the file viewer.

`annorex rex agents` prints the agent panes of the session as one line of JSON, `{"agents":[...]}`. Each pane has a
`block_id`, an `agent`, a `title`, a `subtitle` (the work directory), and a `detail` (the window). The Lua module uses
this command for [The agent picker](#the-agent-picker).

`annorex deliver` follows the steps in [How a send works](#how-a-send-works). With `--paste-only`, it pastes the text
and does not press Enter. It refuses to send when:

- no supported agent runs in the pane,
- the agent shows a dialog, such as a permission prompt or a question,
- the agent process does not own its Rex status record (the agent did not report its state yet), or
- the pane did not turn on bracketed paste.

If the agent opens a dialog after the paste, `annorex deliver` does not press Enter. The text stays in the input box of
the agent. Answer the dialog, then press Enter yourself.

When the agent is busy, Claude Code queues the message. Pi, OpenCode, and Codex CLI use it to steer the current task.

Exit codes:

| Code | Meaning                                                                                                     |
|------|-------------------------------------------------------------------------------------------------------------|
| 0    | Sent, pasted, probe complete, or the viewer closed                                                          |
| 1    | Another failure, for example a file or the output that Annorex cannot write                                 |
| 3    | Refused: no agent (not `annorex rex open`), a dialog, no agent status, or no bracketed paste. Or Enter held |
| 4    | A Rex call failed, or the `rex` binary was not found                                                        |
| 5    | `annorex rex open` only: the target pane runs no agent                                                      |
| 64   | Usage error: a bad argument, no session, or an empty message                                                |

## Troubleshooting

- The key does nothing. Make sure that the setup lines are in `init.lua`. A shortcut that you set in the Rex app wins
  over `init.lua`. Remove it or use another key. Then run `rex config reload`.
- "annorex was not found; tried: ...". The binary is not in one of the places that [Lua options](#lua-options) lists.
  Run `annorex rex install` again with the binary that you use. Or set the path with `bin =` in the `setup` call.
- "the annorex binary ... from setup{} does not exist". Correct the path in `bin =`, or remove `bin =`.
- "the rex binary was not found". Annorex looks for `rex` in `REX_BIN_DIR`, then in the Rex process that started it,
  then in `PATH`. Update Annorex.
- "Not sent: the agent has not reported its state yet". The agent is starting, or it shows a prompt that it does not
  report, for example the Claude Code trust prompt. Wait until the agent is ready. Then press `S` again. Annorex keeps
  your notes.
- "The agent shows a dialog". Answer the dialog in the agent pane. Then press `S` again.
- "Pasted, but Enter was held". A dialog opened after the paste. Answer it. Then press Enter in the agent pane.
- `alt+enter` does not add a new line. Set the Option key to send Alt in the Rex settings. Or use `ctrl+j`.
- The pane zoom ends when Annorex opens. This is Rex behavior. Rex does not restore the zoom when you close the viewer.
- A sideways swipe hides the Rex sidebar. Turn off the Rex setting "Use a gesture to toggle the sidebar".
- A drag does not select text for Rex copy. Annorex uses the mouse, so a drag selects text in Annorex. Annorex has no
  key that copies the selected text. `Y` copies the notes. Each note contains its quoted text.
- Annorex does not find the agent in a pane that started the agent with `exec` from a login shell. Version 0.1.0 has
  this fault. Update to version 0.2.0 or later.
- "pane too small". Make the window larger than 40 columns and 10 rows.

## Known limits

- Claude Code asks whether to trust a folder when it starts in a new folder. Claude Code does not report this dialog to
  Rex. Annorex cannot see it. Do not send while this dialog is open.
- At the first start of Codex in a new folder, Codex asks whether you trust the folder. Do not send notes while this
  question shows. Rex can report Codex as idle, or report no state, at that time. With no state, Annorex refuses to
  send. With idle, Annorex would paste the notes into the question.
- If the input box of the agent has text when you send, the notes join that text. Annorex cannot see the input box.
- Annorex shows a change of a file only when the size or the time of the last change differs. A change that keeps
  both is not seen.

Supported agents: Claude Code, Pi, OpenCode, and Codex CLI, in Rex on macOS.

## How Annorex differs from plannotator-tui

plannotator-tui is a separate project with its own scope. It now has its own Rex integration, "Rex Annotate" (keys
Shift+Cmd+O). These points describe Annorex only:

- Annorex works only in Rex. It opens one full-window layer in each Rex tab and never opens a
  second one in the same tab.
- Annorex shows the replies and the files of the agent in one viewer. One send gives the notes on all of them to the
  agent in one message.
- The file tree puts the files that the agent changed at the top.
- Annorex keeps a draft of each note on disk. A closed viewer or a restart does not lose a note that you did not send.
  After the agent got the message, Annorex deletes the sent notes from the disk.
- Annorex shows a banner when a file changes on disk while you annotate it.
- Annorex tests the state of the agent before it sends. It refuses when the agent shows a dialog or did not report its
  state. It keeps your notes.

## How Annorex connects to Rex

Rex does not have a plugin system. Rex has custom actions in its Lua configuration file. Annorex has two parts:

1. A small Lua module. The module adds the actions "review the last reply" and "review files" to the Rex command
   palette.
2. A Rust program. The program runs in the pane that the action opens.

For more information, read [ARCHITECTURE.md](ARCHITECTURE.md).
