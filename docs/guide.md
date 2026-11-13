# Annorex guide

This guide is the full reference for Annorex. It lists all keys, layouts, commands, and settings. For a short start, read
the [README](../README.md). For the design, read [ARCHITECTURE.md](ARCHITECTURE.md).

## Contents

- [What Annorex does](#what-annorex-does)
- [Install](#install)
- [Review the last reply](#review-the-last-reply)
- [Help screen](#help-screen)
- [Annotate and send](#annotate-and-send)
- [Review files](#review-files)
- [Drafts](#drafts)
- [The config file](#the-config-file)
- [Use the commands](#use-the-commands)
- [Known limits](#known-limits)
- [How Annorex connects to Rex](#how-annorex-connects-to-rex)

## What Annorex does

- **One viewer for replies and files.** A Rex action opens Annorex in a floating layer over the full window of the
  agent. Annorex lets you choose a reply, and shows it as formatted Markdown. You can select one of the 25 newest replies.
  The layout follows the size of the window. A second Rex action starts the same viewer on the files of the work
  directory of the agent. A tree and a fuzzy find help you to open files. You can open many files and annotate each one.
- **Notes.** Select text in a reply or in a file. Add a comment, mark the text for removal, or give a replacement.
- **Send, paste, and copy.** Annorex exports your notes as Markdown. It sends them to the agent, pastes them without
  Enter, or copies them. A send gives only the notes that Annorex did not send before. The notes of all replies and
  open files go in one message.
- **Drafts.** Annorex saves your notes to disk after each change.
- **New reply banner.** When the agent answers while Annorex is open, a banner says `The agent has a new reply. Press r
  to read it.` Annorex looks at the transcript every 2 seconds.
- **Change banner.** When the agent changes an open file, a banner shows. Press `r` to read the file again. Your notes
  move to their text.
- `annorex probe` shows the agent in a pane, its state, and whether Annorex can send to it.
- `annorex deliver` sends text from standard input to the agent in a pane. It removes escape characters and control
  characters, pastes the text as one bracketed paste, and presses Enter.

The tests show that a multi-line Markdown message arrives intact in Claude Code, Pi, and OpenCode, also when the agent
is busy.


## Install

```sh
cargo install --path crates/annorex
annorex rex install
```

`annorex rex install` writes the Lua module `annorex.lua` beside the `init.lua` of Rex. It does not change `init.lua`.
Use `--dir DIR` to write the module to a different directory.
The command then runs `rex config check` on a scratch copy of the module. It reports errors in the module.
Then add these lines to `init.lua`:

```lua
require("annorex").setup{}
rex.bind("cmd+shift+a", "annorex_last")
rex.bind("cmd+shift+f", "annorex_files")
```

Run `rex config reload`.

Rex runs actions with a short `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), not the `PATH` of your shell. So the module
looks for the binary itself. It uses the first file that exists:

1. The binary that ran `annorex rex install`. The command prints this path after `Rex starts:`.
2. `$CARGO_HOME/bin/annorex`, else `~/.cargo/bin/annorex`.
3. `/opt/homebrew/bin/annorex`, then `/usr/local/bin/annorex`.
4. `~/.local/bin/annorex`.

If the binary is in a different place, give its path:

```lua
require("annorex").setup{ bin = "/path/to/annorex" }
```

Annorex always opens over the full window of the agent. The agent pane keeps its size. There is no option for the
placement. The key `placement` in `config.toml` and the option `--placement` give an error.

Only one Annorex runs in each Rex session. If you start Annorex while one is open in the session, Annorex shows the open
one. It does not open a second one, also for another agent. Different sessions can each run their
own Annorex at the same time. Rex ends the zoom of a pane when the layer opens.
Rex does not restore the zoom when you close the viewer.
If the open Annorex is for another agent, the action shows a message that names it.
Press `q` there to close it.

The replies of the agent and the files of its work directory are in the same Annorex. `annorex_last` and
`annorex_files` open the same viewer. They differ in the first document: the newest reply, or the files that you
name (the file tree when you name none). The file viewer shows the tree on the left and the file keys in the footer.
The last-reply viewer hides the tree until you press `T`. With two or more replies, it opens the reply list first. The newest reply is selected. Press `enter` to open the selected reply. Press `esc` to quit Annorex. The footer shows the keys of the document on the screen: reply keys for a reply, file keys for a file. All keys work in both viewers. One send gives the notes on all replies and files to the agent.

## Review the last reply

Focus the pane of Claude Code, Pi, or OpenCode. Press the key that you bound, or run "Annorex: review the last reply"
from the command palette. The action `annorex_last` opens the viewer over the full window. If an Annorex is already
open in the session, the action focuses it.

The header shows the agent and its state, for example `Claude Code (idle)` or `OpenCode (waiting for permission)`. Annorex
reads the state again every 2 seconds.

Annorex has two modes in the document. Block mode is the default. Text mode is for notes on a part of a block. To enter text mode, press `v`, `V`, or a move key such as `h` or `l`, or drag the mouse over text.

### Block mode

At the start, no block has the focus, and the action bar does not show. Press `j` to focus the first block on the
screen. Press `k` to focus the last block on the screen. A click on a block also focuses it. A note key without a focus
shows `choose a block first (j/k or a click)`. Press `esc` to remove the focus. A new document starts without a focus.
After a reload, the same document keeps its focus, or keeps no focus.

With a focus, one block has it. A block is a heading, a paragraph, a list item (a nested item is its own block), a whole code
block, a whole table, a whole quote, or a rule. In a plain text or code file, a block is a group of lines between blank
lines. The focused block has a bar in the left margin. The note keys act on it. Press `enter` to show the action bar
above it.

| Key                     | Function                                                         |
|-------------------------|------------------------------------------------------------------|
| `j` / `k`, arrows       | Go to the next or the previous block                             |
| `g` / `G`, `home` / `end` | Go to the first or the last block. Without a focus, only scroll to the top or the end |
| `ctrl+d` / `ctrl+u`     | Scroll half a page, then focus the first block in the view. Without a focus, only scroll |
| `PageDown` / `PageUp`   | Scroll one page, then focus the first block in the view          |
| `o` / `d`               | Note on the block: looks good or remove (saves at once, no text, no note box) |
| `c` / `s`               | Note on the block: comment or replace (opens the note box)         |
| `e` / `x`               | Edit or delete the note of the block                             |
| `enter`                 | Show the action bar for the whole block. Block mode stays. Without a focus, focus the first block on the screen first |
| `v`                     | Enter text mode and start a selection at the start of the block  |
| `V`                     | Enter text mode with the lines of the block selected             |
| `h` `l` `w` `b` `0` `$` | Enter text mode at the start of the block, then do the move      |

A block that is taller than the screen scrolls first. `j` moves to the next block when the end of the block is visible.
`k` works the same way upward. If several notes share one block, a numbered list shows. Press `1` to `9` to pick one.

### Text mode

Text mode has a free cursor and a character selection.

| Key                     | Function                                                         |
|-------------------------|------------------------------------------------------------------|
| `j` / `k`, arrows       | Move down or up one line                                         |
| `h` / `l`               | Move left or right one character                                 |
| `w` / `b`               | Move to the next or the previous word                            |
| `0` / `$`               | Move to the start or the end of the line                         |
| `ctrl+d` / `ctrl+u`     | Move down or up half a page                                      |
| `PageDown` / `PageUp`   | Move down or up one page                                         |
| `g` / `G`               | Go to the start or the end                                       |
| `v` / `V`               | Start a selection of characters or of lines. Press the key again to end the selection and show the action bar. A third press clears the selection |
| `o` / `c` / `d` / `s`   | Note on the selection, or on the line of the cursor: looks good, comment, remove, or replace |
| `esc`                   | Clear the selection. Without a selection, go back to block mode. In block mode, remove the focus. With nothing selected or focused, quit |
| `n` / `N`               | Go to the next or the previous match (in text mode)              |

### The action bar

The action bar shows after you finish a selection:

- when a drag of the mouse ends,
- after a double click on a word,
- after `enter` on the focused block,
- after the `v` or `V` that ends a selection.

After `enter`, the selection is the whole focused block. The action bar shows on the row above the selection. When
there is no row above, it shows on the row below. It starts at the first column of the selection. Near the right edge,
it moves left to stay in the document. It has four items:
`o ✓ looks good`, `c 💬 comment`, `d ✕ remove`, and `s ⇄ replace`. Each item has the icon and the color of its note
card. A click on an item does the same as its key.

The keys `o`, `c`, `d`, and `s` also work without the action bar. A focus move with `j` or `k` does not show the action
bar. Each key in the document closes it, except `?`: the help screen only covers it. A mouse click outside the action
bar closes it. The mouse wheel does not close it. A saved note and an open note box close it too.

### Keys in both modes

| Key                     | Function                                                         |
|-------------------------|------------------------------------------------------------------|
| `L`                     | Edit the note at the cursor, or add a comment                   |
| `a`                     | Move the focus to the list of notes                              |
| `S`                     | Send the notes to the agent                                      |
| `M`                     | Write a general note, then send                                  |
| `P`                     | Paste the notes into the agent input box, without Enter          |
| `Y`                     | Copy the notes                                                   |
| `X`                     | Discard the notes of the document on the screen (asks first)     |
| `/`                     | Find text (the case is ignored)                                  |
| `n` / `N`               | Go to the next or the previous match                             |
| `p`                     | Open or close the list of replies and open files                 |
| `tab`                   | Move the focus between the tree, the document, and the card rail |
| `r`                     | Read the transcript again                                        |
| `?`                     | Open the help screen                                             |
| `q`, `ctrl+c`           | Close the viewer                                                 |

In the list of notes and on a card, `j` and `k` move between notes, `enter` goes to the note, and `e` and `x` edit
and delete the note. `esc` returns to the document. In the list of replies, `enter` shows the selected reply.

### The note box

The note box is a floating box below the block, or above it when there is no room below. The title names the kind and
the line, for example `Comment · line 20`. An empty box shows a dim hint. It has the buttons `save` and `cancel`.

`enter` saves the note. A new line needs `shift+enter` when the terminal supports it. Otherwise, use `ctrl+j`. The
footer shows the key that works. `alt+enter` also adds a new line. `esc` cancels.

The box shows at most 8 rows of text. It follows the end of the text. Press `PgUp` or `PgDn`, or use the mouse wheel
over the box, to scroll. A new character goes back to the end. A comment needs text. The notes `looks good` and `remove` have no text,
so they save at once without the box. `e` on them shows `Looks good has no text.` or `Remove has no text.` A draft
of an older version can have text on these notes. Annorex keeps that text in the export.

The config file can still have `labels`. Annorex reads them and does not use them. A note with a label keeps its label
in the export.

### The layout

The layout changes with the size of the window:

- 120 columns or more: the file tree on the left (when it shows), the document, and the card rail on the
  right. The tree is 28 columns wide. The rail is 34 columns wide. Each note has a card in the rail.
  The document takes all the room between the tree and the rail.
- 100 to 119 columns: the file tree on the left (when it shows) and the document. There is no rail. A dot in the color of the note
  marks the note in the right margin of the block.
- Less than 100 columns, or less than 16 rows: one column. The tree and the list of notes open over the document.
  The dots also show in the right margin when the document is 21 columns or more wide.
- Less than 40 columns or less than 10 rows: Annorex shows "pane too small".

The file viewer starts with the tree. With no path, it shows no document until you open a file. Press `p` to open a reply. The last-reply viewer starts without the tree. `T` shows the tree with the focus.
A second `T` hides it. Over a narrow window, `T` opens and closes the tree.
The list of replies (`p`), the file finder (`ctrl+p`), and the session picker open in a box in the center, at all widths.

The header shows the view, the agent and its state, and the position. It adds `unsent in reply 1,3` or `unsent in 2 files` when notes are not sent. On the right, it has the send button
`[ Send N → agent ▸ ]`. With no notes, the button is dim: `[ Send ▸ ]`. It is dim while the agent shows a dialog too. While the agent works, the button adds
`queued` for Claude Code or `steers` for Pi and OpenCode. After a send, it shows `[ Sent HH:MM ]`. The footer shows the
keys of the current mode. Annorex leaves out the hints that do not fit, from the end. `? help` stays while it fits.

The note colors are the 16 ANSI colors. The text under a note keeps its color. Looks good has a green underline. A
comment has a yellow underline. Remove is red and crossed out. Replace is magenta and crossed out. A bar in the note
color shows in the left margin of each row under a note. The cyan focus bar shows on the focused block instead.

A table that is wider than the window gets narrower columns, and its cells wrap. Each column keeps its longest word, up
to 20 columns. If even these widths do not fit, Annorex clips the lines of the table.

### The mouse

Each mouse action has a key. The help screen does not list the mouse actions.

| Where                      | Action       | Result                                                                  |
|----------------------------|--------------|-------------------------------------------------------------------------|
| List, document, tree, rail | Wheel        | Scroll the part under the pointer by 3 lines. The focus does not change |
| Block                      | Click        | Focus the block (block mode)                                            |
| Document text              | Drag         | Select characters (text mode). At the top or the bottom row, the document scrolls. The action bar shows at the release |
| Document text              | Double click | Select the word and show the action bar                                 |
| Action bar item            | Click        | Add that note. Looks good and remove save at once. The others open the note box   |
| Card                       | Click        | Focus the note and its block. The card shows `edit` and `remove` buttons. Looks good and remove have `remove` only |
| Header send button         | Click        | The same as `S`, with the same dialog check and messages                |
| Tree row                   | Click        | Open the file, or open or close the directory                           |
| Row of a list or picker    | Click        | Open that reply, session, or file                                       |
| Numbered list of notes     | Click        | Choose that note                                                        |
| Footer hint                | Click        | Do that action                                                          |
| Note box                   | Click `save` or `cancel` | Save or cancel. A click outside the box does nothing. The wheel over the box scrolls it |
| Help screen                | Click or wheel | Close the help screen                                                 |

A question in the footer (`y/n`) and the find input take no mouse event. There is no right click and no hover action.
Annorex has no key that copies the selected text. `Y` copies the notes with their quoted text.

Pi and OpenCode can have more than one session in the same directory. If Annorex cannot find the session of the agent
with certainty, the list shows the sessions, newest first. Select one with `enter`. If Annorex finds no replies (for
example, the agent has not answered yet), the list says why, and `r` looks again. The files still work.

## Help screen

Press `?` to open a help screen. It lists all keys from the live key map, so it shows your changes from `config.toml`.
Press any key to close it. The config action name is `help`. The footer shows `? help`.

## Annotate and send

1. Move to the block with `j` and `k`. To note a part of a block, press `v` and move to select characters, or press
   `V` to select lines. Without a selection in text mode, the note uses the line of the cursor.
2. Press `o` to mark the text as good. This needs no text. Press `c` to comment, `d` to mark the text for removal, or
   `s` to replace it. Type the note. Press `enter`.
3. Press `S` or click the send button in the header. The button shows `[ Send N → agent ▸ ]`, with N the number of
   notes. After a send that works, Annorex saves the drafts as sent and closes. If the send fails, Annorex stays open
   and the footer shows the reason.

The export is Markdown. It starts with `# Annotations on last reply`. For an older reply, the name is
`earlier reply (HH:MM)` in your local time. Each note has a number, the line numbers, the quoted source text, and
your note as a quote. The text before the quote names the type: `Comment on:`, `Remove this:`, or `Looks good:`.

Annorex does not add a second identical note. The note must match in kind, range, label, and text. The footer says
`Already saved.`

A send gives only the notes that Annorex did not send before. After you change a sent note, it counts as new. `Y`
always copies all notes.

`S` refuses to send when the agent shows a dialog, such as a permission prompt. The footer says
`Not sent: the agent shows a permission dialog. Your notes are kept.` Your notes stay in the draft.

Annorex sends and pastes only when the agent process owns its Rex status record. Otherwise it refuses. The footer says
`Not sent: the agent has not reported its state yet; wait until it is ready. Your notes are kept.` This happens while
the agent starts or shows a prompt that it does not report. Wait until the agent is ready, then press `S` again. If the
agent opens a dialog after the paste, Annorex does not press Enter. The footer says `Pasted, but Enter was held`. Answer the dialog, then press Enter in the
agent pane. When the agent is busy, Claude Code queues the message. Pi and OpenCode use it to steer the current task.

`Y` copies the notes with an OSC 52 sequence and with `pbcopy`. If only the OSC 52 part works, the footer says so.

`q` quits when no note box or text input is open. In a box, `q` types a letter. `esc` steps back one level: it closes a box, then clears the selection, then removes the focus.
When nothing is selected or focused, `esc` quits too. The footer ends with `q quit`. It shows `esc unselect` only while
something is selected or focused. `P` and `Y` do not close Annorex.

`q`, `esc` with nothing selected, and `ctrl+c` with unsent notes ask what to do: send (`s`), keep the draft (`k`), discard (`d`), or go back
(`esc`).

## Review files

Focus the pane of Claude Code, Pi, or OpenCode. Press the key that you bound, or run "Annorex: review files" from the
command palette. The action `annorex_files` opens the file viewer over the full window. If an Annorex is already open
in the session, the action focuses it. Then Annorex ignores the new paths. The root is the work directory of the agent process.

The viewer starts with the file tree. Open a file with `enter`. Annorex shows a Markdown file (`.md` or `.markdown`) as
Markdown, without line numbers. It shows all other files as plain text with line numbers. It shows the text of a file with control characters
removed.

You can open up to 50 files. Each file has its own notes and its own draft. The keys for replies work in a file
too. These keys are for files:

| Key        | Function                                                                         |
|------------|----------------------------------------------------------------------------------|
| `T`        | Hide or show the file tree (over a narrow window: open or close it)             |
| `ctrl+p`   | Open the fuzzy find of files                                                     |
| `p`        | Open or close the list of replies and open files (`●` marks unsent notes)        |
| `[` / `]`  | Show the previous or the next document (replies first, then the open files)      |
| `W`        | Close the file on the screen (asks first when it has unsent notes)               |
| `r`        | Read the file again, and look for new replies                                    |

In the tree, `j` and `k` move, `l` and `h` open and close a directory, `enter` opens a file or a directory, and `/`
starts a filter. In the fuzzy find, type to filter by path, move with the arrows or with `ctrl+n` and `ctrl+p`, and
press `enter` to open the file. A `●` marks a file with a saved draft that has unsent notes.

The tree obeys `.gitignore`, also outside a git repository. It skips hidden files and the `.git` directory. It does not
follow links. Annorex refuses a file that is outside the work directory, also through a link.

Annorex looks at the open files every 2 seconds. When a file changes on disk, a banner shows
`<path> changed on disk. Press r to read it again.` The list marks the file with `≠`. Press `r`. The text is new, and
each note moves to its quoted text. If the text is gone, the note stays and the export says `text not found`. When the
agent deletes a file, the banner says `<path> was deleted. Your notes are kept.` and the text stays.

The tree puts the files that the agent changed since your last prompt at the top, under the line `* changed by the agent: N`,
and marks them with `*`. Annorex reads the transcript of the agent: the files of its successful `Edit`, `Write`, `edit`,
`write`, and `apply_patch` calls. The fuzzy find lists them first too. When the agent changes more files, a banner says
`The agent changed N more files.` until you open the tree. The list is a hint, not the full list. A file that the agent
changed with a shell command (for example `sed -i`) or with a tool of an MCP server is not shown. Neither is a file
that the agent wrote and deleted, or that is outside the work directory. Your own send is a prompt: after it, the list
holds only what the agent changes next (raise `changes_turns` to look back further). Annorex follows the newest session of
the agent in the work directory. With two Pi or OpenCode panes in one directory, the list can belong to the other pane.

One send gives the general notes and the notes of all documents in one message. It has one section for each
document: `# Annotations on last reply` (or `earlier reply (HH:MM)`) for a reply, and `# Annotations on PATH` for a
file. The replies come first. The files follow in the order in which you opened them. If you press `q` with unsent
notes, the question counts the documents.

You can give files at the start. The paths are relative to the work directory:

```sh
annorex files --target block:... README.md docs/guide.md
annorex rex open files --target block:... README.md
```

A path can name a directory. Then the tree opens there. If an Annorex is already open in the session,
`annorex rex open files` only moves the focus to it and does not open the paths. Open them in that viewer with `ctrl+p`
or `T`.

Limits: Annorex does not open a file larger than 1 MiB or a file with a NUL byte in its first 8192 bytes. The tree
shows the first 20,000 entries and says so when there are more. If you open more than 50 files, Annorex asks you to
close one first.

## Drafts

Annorex saves one draft for each reply and for each file in `$XDG_STATE_HOME/annorex/drafts`. The default is
`~/.local/state/annorex/drafts`. The directory has mode `0700` and the files have mode `0600`. The draft keeps the open
note box, so a restart does not lose half a note. If a draft file is unreadable, Annorex keeps it and shows a message.

## The config file

Annorex reads `$XDG_CONFIG_HOME/annorex/config.toml`, or `~/.config/annorex/config.toml`. All settings are optional.

```toml
enter_delay_ms = 300       # 50 to 5000: the wait between the paste and Enter
labels = ["nit"]           # read, but the note box does not use labels
changes_turns = 1          # 1 to 20: how many of your prompts the tree looks back for changed files

[keys]
comment = "y"              # an action name and a key
```

The `[keys]` table can bind the action `help` too. The viewer exits when its terminal closes.

A bad config file stops Annorex with exit code 64 and a message. The keys `enter`, `esc`, `tab`, the arrows, the page
keys, `home`, `end`, and `ctrl+c` are fixed.

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
```

`annorex last` runs the viewer in the current terminal. `annorex rex open last` opens the layer with the viewer, as the
action does. It prints the block ID of the viewer as JSON. If an Annorex is already open, the JSON has the field
`existing`. `annorex files` and `annorex rex open files` do the same for
the file viewer.

`annorex deliver` refuses to send when:

- no supported agent runs in the pane,
- the agent shows a dialog, such as a permission prompt or a question,
- the agent process does not own its Rex status record (the agent has not reported its state yet), or
- the pane has not turned on bracketed paste.

If the agent opens a dialog after the paste, `annorex deliver` does not press Enter. The text stays in the input box of
the agent. Answer the dialog, then press Enter yourself.

When the agent is busy, Claude Code queues the message. Pi and OpenCode use it to steer the current task.

Exit codes:

| Code | Meaning                                                                    |
|------|----------------------------------------------------------------------------|
| 0    | Sent, pasted, probe complete, or the viewer closed                         |
| 1    | Another failure, for example a file that Annorex cannot write              |
| 3    | Refused: no agent, a dialog, no agent status, or no bracketed paste; or Enter held          |
| 4    | A Rex call failed, or the `rex` binary was not found                       |
| 64   | Usage error: a bad argument, no session, or an empty message               |

## Known limits

- Claude Code asks whether to trust a folder when it starts in a new folder. Claude Code does not report this dialog to
  Rex. Annorex cannot see it. Do not send while this dialog is open.
- If the input box of the agent has text when you send, the notes join that text. Annorex cannot see the input box.
- Annorex shows a change of a file only when the size or the time of the last change differs. A change that keeps
  both is not seen.

Supported agents: **Claude Code**, **Pi**, and **OpenCode**, in Rex on macOS.

## How Annorex connects to Rex

Rex does not have a plugin system. Rex has custom actions in its Lua configuration file. Annorex has two parts:

1. A small Lua module. The module adds the actions "review the last reply" and "review files" to the Rex command
   palette.
2. A Rust program. The program runs in the pane that the action opens.

For more information, see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
