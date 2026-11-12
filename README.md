# Annorex

Annorex is a review tool for coding agents in the [Rex](https://www.superlogical.com/rex) terminal. You read the
replies and the files of an agent. You annotate the text. Then Annorex sends your notes back to the agent. It works with
Claude Code, Pi, and OpenCode on macOS.

> **Status: pre-release, private.** The review of replies and files works. Annorex is not published yet.

## Requirements

- Rex on macOS (Apple silicon)
- Claude Code, Pi 1.5.0 or later, or OpenCode 2.x
- Rust 1.99 or later, to build from source

## Install

```sh
cargo install --path crates/annorex
annorex rex install
```

`annorex rex install` writes the Lua module `annorex.lua` beside the `init.lua` of Rex. It does not change `init.lua`.
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

## Quickstart

1. Focus the pane of Claude Code, Pi, or OpenCode.
2. Press `cmd+shift+a` to review the last reply. Press `cmd+shift+f` to review the files.
3. Move from block to block with `j` and `k`. A block is a paragraph, a list item, a heading, a code block, or a table.
   The action bar shows below the block.
4. Press `o` to mark the block as good, `c` to comment, `d` to mark it for removal, or `s` to replace it. `o` needs no
   text. The other keys open an editor. Type the note. Press `enter`.
5. To annotate a part of a block, press `v` and move to select text. This is text mode. Press `esc` twice to go back
   to block mode.
6. Press `S` to send all notes to the agent.

In the viewer, press `?` to see all keys.

## The action bar and notes

The block with the focus has a bar in the left margin. Below it, the action bar shows four buttons:
`[o looks good] [c comment] [d remove] [s replace]`. A click on a button does the same as its key.

The export of `o looks good` has the text `Looks good:` and the quote. The note needs no text. Press `e` later to add
text.

Notes show in place. Looks good has a green background. A comment is underlined in yellow. Remove is red and crossed
out. Replace is magenta and crossed out. At 120 columns or more, each note has a card in a rail on the right. At 100 to
119 columns, a dot in the note color marks the note in the right margin.

The editor opens as a floating box near the block. It has the buttons `save` and `cancel`.

Press `e` to edit and `x` to delete the note at the focus.

## Layout

The layout follows the size of the window.

- 120 columns or more: the file tree (when there are files) on the left, the document, and the card rail on the right.
- 100 to 119 columns: the file tree on the left and the document. There is no card rail.
- Less than 100 columns, or less than 16 rows: one column. The tree and the lists open over the document.
- Less than 40 columns or less than 10 rows: Annorex shows "pane too small".

The header shows the agent and the send button `[ Send N → agent ▸ ]`. The footer shows the keys of the current mode.
The list of replies (`p`), the file finder (`ctrl+p`), and the session picker open in a box in the center.

## Key cheatsheet

| Key                  | Function                                                                 |
|----------------------|--------------------------------------------------------------------------|
| `j` / `k`            | Block mode: go to the next or the previous block                         |
| `g` / `G`            | Go to the first or the last block (the start or the end in text mode)    |
| `ctrl+d` / `ctrl+u`  | Scroll half a page                                                       |
| `v` / `V`            | Select characters or lines (text mode)                                   |
| `h` `l` `w` `b`      | Move by character or by word (text mode)                                 |
| `o`                  | Note: looks good (no text needed)                                        |
| `c` / `d` / `s`      | Note: comment, remove, or replace                                        |
| `e` / `x`            | Edit or delete the note at the focus                                     |
| `esc`                | Clear the selection. A second press goes back to block mode              |
| `S`                  | Send the notes to the agent                                              |
| `M`                  | Write a general note, then send                                          |
| `P`                  | Paste the notes into the agent input box, without Enter                  |
| `Y`                  | Copy the notes                                                           |
| `X`                  | Discard the notes of the document on the screen (asks first)             |
| `a`                  | Move the focus to the list of notes                                      |
| `p`                  | Open or close the list of replies and open files                         |
| `T`                  | Hide or show the file tree                                               |
| `tab`                | Move the focus between the tree, the document, and the card rail         |
| `ctrl+p`             | Find a file by name                                                      |
| `[` / `]`            | Show the previous or the next open file                                  |
| `W`                  | Close the file on the screen                                             |
| `/`                  | Find text                                                                |
| `r`                  | Read the transcript and the files again                                  |
| `?`                  | Show the help screen                                                     |
| `q`                  | Close the viewer                                                         |

For all keys and for the config file, see [docs/guide.md](docs/guide.md).

## Mouse

Each mouse action has a key. The help screen does not list the mouse actions.

| Where                    | Action       | Result                                                                   |
|--------------------------|--------------|--------------------------------------------------------------------------|
| List, document, tree, rail | Wheel      | Scroll the part under the pointer by 3 lines                             |
| Block                    | Click        | Focus the block                                                          |
| Document text            | Drag         | Select characters (text mode)                                            |
| Document text            | Double click | Select the word                                                          |
| Action bar button        | Click        | Add that note. Looks good saves at once. The others open the editor      |
| Card                     | Click        | Focus the note and its block. The card shows `edit` and `remove`         |
| Header `Send` button     | Click        | The same as `S`                                                          |
| Tree row                 | Click        | Open the file, or open or close the directory                            |
| Row of a list or picker  | Click        | Open that reply, session, or file                                            |
| Footer hint              | Click        | Do that action                                                           |
| Editor                   | Click `save` or `cancel` | Save or cancel. A click outside the editor does nothing      |
| Help screen              | Click        | Close the help screen                                                    |

## Theme

Annorex uses the 16 ANSI colors of the terminal. It follows the Rex theme, light or dark. Rex gives programs no way to
read the theme.

## Troubleshooting

- **The key does nothing.** Check that the setup lines are in `init.lua`. A shortcut that you set in the Rex app wins
  over `init.lua`. Remove it or use another key. Then run `rex config reload`.
- **"annorex was not found; tried: ...".** The binary is not in one of the places that Install lists. Run
  `annorex rex install` again with the binary that you use. Or set the path with `bin =` in the `setup` call.
- **"the annorex binary ... from setup{} does not exist".** Correct the path in `bin =`, or remove `bin =`.
- **"the rex binary was not found".** Annorex looks for `rex` in `REX_BIN_DIR`, then in the Rex process that started
  it, then in `PATH`. Update Annorex.
- **"Not sent: the agent has not reported its state yet".** The agent is starting, or it shows a prompt that it does
  not report, for example the Claude Code trust prompt. Wait until the agent is ready. Then press `S` again. Annorex
  keeps your notes.
- **"The agent shows a dialog".** Answer the dialog in the agent pane. Then press `S` again.
- **"Pasted, but Enter was held".** A dialog opened after the paste. Answer it. Then press Enter in the agent pane.
- **`alt+enter` does not add a new line.** Set the Option key to send Alt in the Rex settings. Or use `ctrl+j`.
- **The pane zoom ends when Annorex opens.** This is Rex behavior. Rex does not restore the zoom when you close the
  viewer.
- **A sideways swipe hides the Rex sidebar.** Turn off the Rex setting "Use a gesture to toggle the sidebar".
- **Drag does not select text for Rex copy.** Annorex uses the mouse, so a drag selects text in Annorex. Annorex has no
  key that copies the selected text. `Y` copies the notes. Each note contains its quoted text.
- **"pane too small".** Make the window larger than 40 columns and 10 rows.

## How Annorex differs from plannotator-tui

plannotator-tui is a separate project with its own scope. It now has its own Rex integration, "Rex Annotate" (keys
Shift+Cmd+O). These points describe Annorex only:

- Annorex works only in Rex. It opens one full-window layer in each Rex session and never opens a second one.
- Annorex shows the replies and the files of the agent in one viewer. One send gives the notes on all of them to the
  agent in one message.
- The file tree puts the files that the agent changed at the top.
- Annorex keeps a draft of each note on disk. A closed viewer or a restart does not lose a note.
- Annorex shows a banner when a file changes on disk while you annotate it.
- Annorex checks the agent before it sends. It refuses when the agent shows a dialog or has not reported its state. It
  keeps your notes.

## Documentation

- [docs/guide.md](docs/guide.md): all keys, layouts, commands, and settings.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): the design of Annorex.

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

The idea of the full-window layer comes from herdr-annotate and memex, which both cover the screen. Annorex adds the
rule of one
review layer for each session. The check of the Lua module with `rex config check` on a scratch copy, and the lookup of
the window of the agent pane with `session.describe_block`, follow the same ideas in plannotator-tui.

Annorex is an independent project. It has no connection to these projects, and they do not endorse it.
