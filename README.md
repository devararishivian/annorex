# Annorex

Annorex is a review tool for coding agents in the [Rex](https://www.superlogical.com/rex) terminal. You annotate the
text that an agent wrote. Then Annorex sends your notes back to the agent.

> **Status: annotate and send available.** Annorex shows the replies of an agent in a pane next to it. You can
> annotate a reply and send the notes back to the agent. The commands `annorex probe` and `annorex deliver` work. The
> review of files is not available yet. The "Planned features" section describes it.

## Available now

- **The reply viewer.** A Rex action opens Annorex in a pane next to the agent. Annorex shows the newest reply of the
  agent as formatted Markdown. You can select one of the 25 newest replies. The layout follows the size of the pane.
- **Annotations.** Select text in a reply. Add a comment, mark the text for removal, or give a replacement.
- **Send, paste, and copy.** Annorex exports your notes as Markdown. It sends them to the agent, pastes them without
  Enter, or copies them.
- **Drafts.** Annorex saves your notes to disk after each change.
- `annorex probe` shows the agent in a pane, its state, and whether Annorex can send to it.
- `annorex deliver` sends text from standard input to the agent in a pane. It removes escape characters and control
  characters, pastes the text as one bracketed paste, and presses Enter.

The tests show that a multi-line Markdown message arrives intact in Claude Code, Pi, and OpenCode, also when the agent
is busy.

### Install

```sh
cargo install --path crates/annorex
annorex rex install
```

`annorex rex install` writes the Lua module `annorex.lua` beside the `init.lua` of Rex. It does not change `init.lua`.
Use `--dir DIR` to write the module to a different directory. Then add these lines to `init.lua`:

```lua
require("annorex").setup{}
rex.bind("cmd+shift+a", "annorex_last")
```

Run `rex config reload`. If Rex cannot find `annorex` on its `PATH`, give the path of the binary:

```lua
require("annorex").setup{ bin = "/Users/me/.cargo/bin/annorex" }
```

The option `placement` sets where the viewer opens: `"auto"` (the default), `"right"`, or `"below"`. With `"auto"`, the
viewer opens on the right when the agent pane has 160 columns or more. Otherwise it opens below.

### Review the last reply

Focus the pane of Claude Code, Pi, or OpenCode. Press the key that you bound, or run "Annorex: review the last reply"
from the command palette. The action `annorex_last` opens the viewer next to the agent pane. If the viewer is already
open for that pane, the action focuses it.

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
| `X`                     | Discard the notes of this reply (asks first)                     |
| `/`                     | Find text (the case is ignored)                                  |
| `n` / `N`               | Go to the next or the previous match                             |
| `p`                     | Open or close the reply list                                     |
| `enter` (in the list)   | Show the selected reply, or go to the annotation                 |
| `e` / `x` (annotation list) | Edit or delete the annotation                                |
| `esc`                   | Cancel the selection, the editor, or the list                    |
| `r`                     | Read the transcript again                                        |
| `z`                     | Zoom the viewer pane, or restore the layout                      |
| `q`, `ctrl+c`           | Close the viewer                                                 |

In the editor, `enter` saves the annotation. `ctrl+j` or `alt+enter` adds a new line. `tab` changes the label.

The layout changes with the size of the pane:

- 100 columns or more: the reply list is a panel on the right side.
- 60 to 99 columns: the reply list is a panel at the bottom.
- Less than 60 columns or less than 16 rows: the reply list covers the reply.
- Less than 40 columns or less than 10 rows: Annorex shows "pane too small".

Pi and OpenCode can have more than one session in the same directory. If Annorex cannot find the session of the agent
with certainty, it shows the sessions, newest first. Select one with `enter`.

### Annotate and send

1. Move to the text. Press `v` and move to select characters, or press `V` to select lines. Without a selection, the
   action uses the line of the cursor.
2. Press `c` to comment, `d` to mark the text for removal, or `s` to replace it. Type the note. Press `enter`.
3. Press `S` to send. The header shows `[Send N ▸]`, with N the number of annotations. After a send, it shows
   `[Sent HH:MM]`.

The export is Markdown. It starts with `# Annotations on last reply`. Each annotation has a number, the line numbers,
the quoted source text, and your note as a quote.

`S` refuses to send when the agent shows a dialog, such as a permission prompt. Your notes stay in the draft. If the
agent opens a dialog after the paste, Annorex does not press Enter. Answer the dialog, then press Enter in the agent
pane. When the agent is busy, Claude Code queues the message. Pi and OpenCode use it to steer the current task.

`Y` copies the notes with an OSC 52 sequence and with `pbcopy`. If only the OSC 52 part works, the footer says so.

`q` with unsent notes asks what to do: send (`s`), keep the draft (`k`), discard (`d`), or go back (`esc`).

### Drafts

Annorex saves one draft for each reply in `$XDG_STATE_HOME/annorex/drafts`. The default is
`~/.local/state/annorex/drafts`. The directory has mode `0700` and the files have mode `0600`. The draft keeps the open
editor, so a restart does not lose half a note. If a draft file is unreadable, Annorex keeps it and shows a message.

### The config file

Annorex reads `$XDG_CONFIG_HOME/annorex/config.toml`, or `~/.config/annorex/config.toml`. All settings are optional.

```toml
placement = "right"        # "auto", "right", or "below"
enter_delay_ms = 300       # 50 to 5000: the wait between the paste and Enter
labels = ["nit"]           # more labels, after question, looks good, and must fix

[keys]
comment = "y"              # an action name and a key
```

A bad config file stops Annorex with exit code 64 and a message. The keys `enter`, `esc`, `tab`, the arrows, the page
keys, `home`, `end`, and `ctrl+c` are fixed. The `--placement` option of the command line has priority over the file.

### Use the commands

Run the commands in a Rex pane. Give the block ID of the agent pane as `--target`. The session is `$REX_SESSION`, or
give it with `--session`.

```sh
annorex probe --target block:...
annorex probe --target block:... --json
printf 'Fix the test in parser.rs.\n' | annorex deliver --target block:...
printf 'Fix the test in parser.rs.\n' | annorex deliver --target block:... --paste-only
annorex last --target block:...
annorex rex open last --target block:... --placement right
```

`annorex last` runs the viewer in the current terminal. `annorex rex open last` opens a pane with the viewer, as the
action does. It prints the block ID of the viewer as JSON.

`annorex deliver` refuses to send when:

- no supported agent runs in the pane,
- the agent shows a dialog, such as a permission prompt or a question, or
- the pane has not turned on bracketed paste.

If the agent opens a dialog after the paste, `annorex deliver` does not press Enter. The text stays in the input box of
the agent. Answer the dialog, then press Enter yourself.

When the agent is busy, Claude Code queues the message. Pi and OpenCode use it to steer the current task.

Exit codes:

| Code | Meaning                                                                    |
|------|----------------------------------------------------------------------------|
| 0    | Sent, pasted, probe complete, or the viewer closed                         |
| 1    | Another failure, for example a file that Annorex cannot write              |
| 3    | Refused: no agent, a dialog, or no bracketed paste; or Enter held          |
| 4    | A Rex call failed, or the `rex` binary was not found                       |
| 64   | Usage error: a bad argument, no session, or an empty message               |

### Known limits

- Claude Code asks whether to trust a folder when it starts in a new folder. Claude Code does not report this dialog to
  Rex. Annorex cannot see it. Do not send while this dialog is open.
- The reply list shows the time of each reply in UTC.
- If the input box of the agent has text when you send, the notes join that text. Annorex cannot see the input box.
- The hints in the footer do not follow keys that you rebind.

## Planned features

- **Review files.** You can open one or more files that the agent wrote. Annorex sends all notes in one message.

Supported agents: **Claude Code**, **Pi**, and **OpenCode**, in Rex on macOS.

## How Annorex connects to Rex

Rex does not have a plugin system. Rex has custom actions in its Lua configuration file. Annorex has two parts:

1. A small Lua module. The module adds the action "review the last reply" to the Rex command palette. The action
   "review files" is planned.
2. A Rust program. The program runs in the pane that the action opens.

For more information, see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Requirements

- Rex on macOS (Apple silicon)
- Claude Code, Pi 1.5.0 or later, or OpenCode 2.x
- Rust 1.99 or later, to build from source

## License

MIT.

## Acknowledgements

These projects are the base of the Annorex idea:

- [Plannotator](https://github.com/backnotprop/plannotator) by backnotprop. Plannotator started the "annotate, then send
  back to the agent" review method.
- [herdr-annotate](https://github.com/plannotator/herdr-annotate) and
  [plannotator-tui](https://github.com/plannotator/plannotator-tui) by Plannotator and Michael Ramos. These projects do
  the same review in a terminal. Annorex uses the transcript readers from plannotator-tui under the MIT License. See
  [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
- [memex](https://github.com/nicosuave/memex) by Nico Ritschel. memex shows how to connect a Rust tool to Rex.

Annorex is an independent project. It has no connection to these projects, and they do not endorse it.
