# Architecture

> **Current state: review of replies and of many files built.** The Rex adapter, agent detection, dialog check,
> payload, delivery, transcript readers, Lua module, install, layer placement, reply loading, document model, wrapping,
> layout, UI, terminal loop, config, keys, review, drafts, send, clipboard, file loading, file tree, fuzzy find, and
> file watch are **built**.

## Overview

```
Rex init.lua ── require("annorex").setup{} ──▶ annorex.lua
                                                  │ custom actions: annorex_last, annorex_files
                                                  ▼ opens a full-window layer
                                     ┌──────────────────────────────────┐
                                     │ annorex (Rust binary)            │──▶ rex CLI (JSON on stdin)
                                     └──────────────────────────────────┘──▶ agent transcripts (read-only)
```

Annorex has two parts:

1. **Lua module** (`annorex.lua`, built). Rex does not have a plugin system. Rex has custom actions in its Lua
   configuration file. The module registers the actions `annorex_last` and `annorex_files`. An action runs
   `annorex rex open last` or `annorex rex open files` with `--target BLOCK --session SESSION` and a
   timeout of 25 seconds. It reports the first line of an error. The module contains no other logic.
2. **Rust binary** (`annorex`, built). The binary does all other work. It uses only the `rex`
   command-line tool to communicate with Rex. It sends the arguments as JSON on standard input. It never opens a
   network port.

## Components

| Component          | State   | Function                                                                                                                                                                             |
|--------------------|---------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Rex adapter        | Built   | Runs `rex block call -s SESSION -b BLOCK METHOD -` and `rex api call -s SESSION METHOD -` with the arguments on standard input. Gives typed results for the process, program status, terminal modes, and size of a pane, and for the session view. Creates, minimizes, and closes layers, and focuses blocks. |
| Agent detection    | Built   | Identifies Claude Code, Pi, or OpenCode from the `app` of the root program status record, when the foreground process confirms it. Otherwise uses `argv0` of the foreground process. |
| Dialog check       | Built   | Refuses a send when the root program status is "blocked" (a permission, question, or authentication dialog). A busy agent can get a message.                                         |
| Payload            | Built   | Removes escape characters and control characters. Keeps line feeds and tabs. Puts the text in one bracketed paste.                                                                   |
| Delivery           | Built   | Checks for a dialog two times, pastes the text, waits 300 ms, checks again, and sends Enter. It never sends again automatically.                                                     |
| Transcript readers | Built   | Find the session of an agent and read its recent messages. Adapted from plannotator-tui (MIT).                                                                                       |
| Install            | Built   | `annorex rex install` writes `annorex.lua` beside the `init.lua` of Rex. It refuses to replace a file that it did not write. It never changes `init.lua`.                           |
| Placement          | Built   | `annorex rex open last` and `annorex rex open files` look for a block with a label that starts with `annorex:` in the session view (the label `annorex-files:` of older versions counts too). If one exists, they raise and focus its layer (one Annorex for each session). Otherwise they create a floating layer over the full window of the agent with `session.new_layer`. If two calls race, the newest layer closes. |
| Reply loading      | Built   | Finds the session of the agent: Claude Code by its process ID, Pi and OpenCode by the directory. Shows a session picker when more than one session can belong to the agent. Reads the 25 newest replies. |
| Document model     | Built   | Shows Markdown with pulldown-cmark (headings, lists, quotes, code, tables, rules). Keeps a map from each displayed line to its source byte range and source line number.            |
| Wrapping           | Built   | Wraps the document at the display width of each character cluster. Keeps the wrapped lines in a cache for each width.                                                               |
| Layout             | Built   | Calculates the areas again for each frame: a side panel at 100 columns or more, a bottom panel at 60 to 99 columns, an overlay below that, and "pane too small" below 40×10.        |
| UI                 | Built   | ratatui screens: the reply, the reply list, find, the session picker, and errors. A resize keeps the top line. The same screens work in a second mode for files: the file list, the tree, the fuzzy find, and the change banner. |
| Terminal loop      | Built   | `annorex last` and `annorex files`. Waits a maximum of 250 ms for an event, applies all queued events, and draws one time. Stops on `q`, on SIGHUP, SIGTERM, or SIGINT, or when the terminal input ends. Reads the agent state every 2 seconds on a thread. |
| Commands           | Built   | `annorex probe`, `annorex deliver`, `annorex last`, `annorex files`, `annorex rex install`, `annorex rex open last`, and `annorex rex open files`. |
| Config             | Built   | Reads `config.toml` (`enter_delay_ms`, labels, keys). A bad file gives exit code 64.                                                                                      |
| Keys               | Built   | Maps each key to an action. Fixed keys cannot be bound. Rejects a key that two actions use.                                                                                          |
| Review             | Built   | Keeps annotations (comment, remove, replace) on source byte ranges. Each annotation has a send time. Exports the notes as Markdown with a fence longer than any backtick run in the quote. For many documents, the export has one section for each: the replies first, then the files in the order of opening. |
| Drafts             | Built   | Saves one JSON file for each reply or file after each change, in an atomic write. Keeps an unreadable file. Moves a quote again when the text moved. Marks it "text not found" if it is gone. |
| Send               | Built   | `S`, `M`, and `P` call delivery with the export. A send gives only the notes that no send took. A dialog gives "Not sent" and keeps the draft. Sets `sent_at` after a send. |
| Clipboard          | Built   | `Y` writes an OSC 52 sequence and runs `pbcopy`. Reports when only the terminal part worked.                                                                                         |
| Files              | Built   | Scans the work directory with the `ignore` crate (`.gitignore` also outside git, no hidden files, no links, 20,000 entries at most). Checks that a path is inside the root, also through links. Loads a file up to 1 MiB and refuses binary data. |
| Fuzzy find         | Built   | Ranks paths with its own scorer: a subsequence match with bonuses for word starts and for consecutive letters. At most 200 results.                                                  |
| One document list  | Built   | `App` has one list of documents: the replies (newest first), then the open files. Each open file has the id `file:PATH`. A problem with the reply session (`ReplySource::Missing` or `Pick`) shows in the reply list and does not block the files. One export and one send cover all documents. The key of a draft uses the session of the agent for a reply and the work directory for a file. Markdown files show as Markdown, other files as plain text. At most 50 files are open.               |
| File tree          | Built   | A panel with the directories, the filter, and the marks for files with unsent drafts. The scan runs on a thread.                                                                      |
| Changed files      | Built   | `annorex-hosts` reads the successful file-changing tool calls of a transcript (`changes`, `claude::changes`, `pi_changes`, `opencode_changes`; OpenCode in both table families). `changes.rs` makes the paths relative to the work directory and a thread polls every 2 seconds. The tree puts the files first and a banner counts the new ones. |
| File watch         | Built   | A thread compares the size and the time of each open file with the values at the last read, every 2 seconds. The viewer shows a banner and places the notes again after `r`.       |

## Crates

- `annorex-hosts` (built): the adapted transcript readers. This crate is separate, so the license boundary is clear.
- `annorex` (built): a library and a binary for all other parts. The Lua module is in `crates/annorex/lua/`.

## Key constraints

- Rust 1.99.1, edition 2024, ratatui 0.30.2, crossterm 0.29.0. The Lua version in Rex is 5.1.
- A Rex custom action must finish in 30 seconds. Thus, the Lua module only opens the pane.
- A pane that Rex creates gets its data from command-line arguments. It does not get environment variables from the
  layout.
