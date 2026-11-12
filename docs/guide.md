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
  agent. Annorex shows the newest reply of the agent as formatted Markdown. You can select one of the 25 newest replies.
  The layout follows the size of the window. A second Rex action starts the same viewer on the files of the work
  directory of the agent. A tree and a fuzzy find help you to open files. You can open many files and annotate each one.
- **Annotations.** Select text in a reply or in a file. Add a comment, mark the text for removal, or give a replacement.
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
`annorex_files` open the same viewer. They differ only in the first document: the newest reply, or the files that you
name (the file tree when you name none). One send gives the notes on all replies and files to the agent.

## Review the last reply

Focus the pane of Claude Code, Pi, or OpenCode. Press the key that you bound, or run "Annorex: review the last reply"
from the command palette. The action `annorex_last` opens the viewer over the full window. If an Annorex is already
open in the session, the action focuses it.

The header shows the agent and its state, for example `claude (idle)` or `opencode (waiting for permission)`. Annorex
reads the state again every 2 seconds.

| Key                     | Function                                                         |
|-------------------------|------------------------------------------------------------------|
| `j` / `k`, arrows       | Move down or up one line                                         |
| `h` / `l`               | Move left or right one character                                 |
| `w` / `b`               | Move to the next or the previous word                            |
| `0` / `$`               | Move to the start or the end of the line                         |
| `ctrl+d` / `ctrl+u`     | Move down or up half a page                                      |
| `PageDown` / `PageUp`   | Move down or up one page                                         |
| `g` / `G`               | Go to the start or the end                                       |
| `v` / `V`               | Start a selection of characters or of lines                      |
| `c` / `d` / `s`         | Annotate: comment, remove, or replace                            |
| `L`                     | Edit the annotation at the cursor, or add a comment              |
| `a`                     | Open or close the annotation list                                |
| `S`                     | Send the notes to the agent                                      |
| `M`                     | Write a general note, then send                                  |
| `P`                     | Paste the notes into the agent input box, without Enter          |
| `Y`                     | Copy the notes                                                   |
| `X`                     | Discard the notes of the document on the screen (asks first)     |
| `/`                     | Find text (the case is ignored)                                  |
| `n` / `N`               | Go to the next or the previous match                             |
| `p`                     | Open or close the list of replies and open files                 |
| `enter` (in the list)   | Show the selected reply, or go to the annotation                 |
| `e` / `x` (annotation list) | Edit or delete the annotation                                |
| `esc`                   | Cancel the selection, the editor, or the list                    |
| `r`                     | Read the transcript again                                        |
| `?`                     | Open the help screen                                             |
| `q`, `ctrl+c`           | Close the viewer                                                 |

In the editor, `enter` saves the annotation. `ctrl+j` or `alt+enter` adds a new line. `tab` changes the label.

The layout changes with the size of the window:

- 100 columns or more: the reply list is a panel on the right side.
- 60 to 99 columns: the reply list is a panel at the bottom.
- Less than 60 columns or less than 16 rows: the reply list covers the reply.
- Less than 40 columns or less than 10 rows: Annorex shows "pane too small".

Pi and OpenCode can have more than one session in the same directory. If Annorex cannot find the session of the agent
with certainty, the list shows the sessions, newest first. Select one with `enter`. If Annorex finds no replies (for
example, the agent has not answered yet), the list says why, and `r` looks again. The files still work.

## Help screen

Press `?` to open a help screen. It lists all keys from the live key map, so it shows your changes from `config.toml`.
Press any key to close it. The config action name is `help`. The footer shows `S send` and `? help`.

## Annotate and send

1. Move to the text. Press `v` and move to select characters, or press `V` to select lines. Without a selection, the
   action uses the line of the cursor.
2. Press `c` to comment, `d` to mark the text for removal, or `s` to replace it. Type the note. Press `enter`.
3. Press `S` to send. The header shows `[Send N ▸]`, with N the number of annotations. After a send, it shows
   `[Sent HH:MM]`.

The export is Markdown. It starts with `# Annotations on last reply`. For an older reply, the name is
`earlier reply (HH:MM)` in your local time. Each annotation has a number, the line numbers, the quoted source text, and
your note as a quote.

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

`q` and `ctrl+c` with unsent notes ask what to do: send (`s`), keep the draft (`k`), discard (`d`), or go back
(`esc`).

## Review files

Focus the pane of Claude Code, Pi, or OpenCode. Press the key that you bound, or run "Annorex: review files" from the
command palette. The action `annorex_files` opens the file viewer over the full window. If an Annorex is already open
in the session, the action focuses it. Then Annorex ignores the new paths. The root is the work directory of the agent process.

The viewer starts with the file tree. Open a file with `enter`. Annorex shows a Markdown file (`.md` or `.markdown`) as
Markdown. It shows all other files as plain text with line numbers. It shows the text of a file with control characters
removed.

You can open up to 50 files. Each file has its own annotations and its own draft. The keys for replies work in a file
too. These keys are for files:

| Key        | Function                                                                         |
|------------|----------------------------------------------------------------------------------|
| `T`        | Open or close the file tree                                                      |
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

One send gives the general notes and the annotations of all documents in one message. It has one section for each
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
editor, so a restart does not lose half a note. If a draft file is unreadable, Annorex keeps it and shows a message.

## The config file

Annorex reads `$XDG_CONFIG_HOME/annorex/config.toml`, or `~/.config/annorex/config.toml`. All settings are optional.

```toml
enter_delay_ms = 300       # 50 to 5000: the wait between the paste and Enter
labels = ["nit"]           # more labels, after question, looks good, and must fix
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
