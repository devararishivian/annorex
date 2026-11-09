# Architecture

> **Current state: viewer built.** The Rex adapter, agent detection, dialog check, payload, delivery, transcript
> readers, Lua module, install, pane placement, reply loading, document model, wrapping, layout, UI, and terminal loop
> are **built**. Annotations, export, the send from the viewer, and the file review are **planned**.

## Overview

```
Rex init.lua ── require("annorex").setup{} ──▶ annorex.lua
                                                  │ custom action: annorex_last (annorex_files planned)
                                                  ▼ opens a split next to the agent pane
                                     ┌──────────────────────────────────┐
                                     │ annorex (Rust binary)            │──▶ rex CLI (JSON on stdin)
                                     └──────────────────────────────────┘──▶ agent transcripts (read-only)
```

Annorex has two parts:

1. **Lua module** (`annorex.lua`, built). Rex does not have a plugin system. Rex has custom actions in its Lua
   configuration file. The module registers the action `annorex_last`. The action runs
   `annorex rex open last --target BLOCK --session SESSION --placement P` with a timeout of 25 seconds. It reports the
   first line of an error. The module contains no other logic.
2. **Rust binary** (`annorex`, built). The binary does all other work. It uses only the `rex`
   command-line tool to communicate with Rex. It sends the arguments as JSON on standard input. It never opens a
   network port.

## Components

| Component          | State   | Function                                                                                                                                                                             |
|--------------------|---------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Rex adapter        | Built   | Runs `rex block call -s SESSION -b BLOCK METHOD -` and `rex api call -s SESSION METHOD -` with the arguments on standard input. Gives typed results for the process, program status, terminal modes, and size of a pane, and for the session view. Creates splits, focuses blocks, and zooms blocks. |
| Agent detection    | Built   | Identifies Claude Code, Pi, or OpenCode from the `app` of the root program status record, when the foreground process confirms it. Otherwise uses `argv0` of the foreground process. |
| Dialog check       | Built   | Refuses a send when the root program status is "blocked" (a permission, question, or authentication dialog). A busy agent can get a message.                                         |
| Payload            | Built   | Removes escape characters and control characters. Keeps line feeds and tabs. Puts the text in one bracketed paste.                                                                   |
| Delivery           | Built   | Checks for a dialog two times, pastes the text, waits 300 ms, checks again, and sends Enter. It never sends again automatically.                                                     |
| Transcript readers | Built   | Find the session of an agent and read its recent messages. Adapted from plannotator-tui (MIT).                                                                                       |
| Install            | Built   | `annorex rex install` writes `annorex.lua` beside the `init.lua` of Rex. It refuses to replace a file that it did not write. It never changes `init.lua`.                           |
| Placement          | Built   | `annorex rex open last` finds the viewer of the agent pane by its label `annorex:BLOCK` and focuses it, or creates a split with `session.new_split`. Right at 160 columns or more, else below. |
| Reply loading      | Built   | Finds the session of the agent: Claude Code by its process ID, Pi and OpenCode by the directory. Shows a session picker when more than one session can belong to the agent. Reads the 25 newest replies. |
| Document model     | Built   | Shows Markdown with pulldown-cmark (headings, lists, quotes, code, tables, rules). Keeps a map from each displayed line to its source byte range and source line number.            |
| Wrapping           | Built   | Wraps the document at the display width of each character cluster. Keeps the wrapped lines in a cache for each width.                                                               |
| Layout             | Built   | Calculates the areas again for each frame: a side panel at 100 columns or more, a bottom panel at 60 to 99 columns, an overlay below that, and "pane too small" below 40×10.        |
| UI                 | Built   | ratatui screens: the reply, the reply list, find, the session picker, and errors. A resize keeps the top line.                                                                       |
| Terminal loop      | Built   | `annorex last`. Waits a maximum of 250 ms for an event, applies all queued events, and draws one time. Stops on `q`, on SIGHUP, SIGTERM, or SIGINT, or when the terminal input ends. Reads the agent state every 2 seconds on a thread. |
| Commands           | Built   | `annorex probe`, `annorex deliver`, `annorex last`, `annorex rex install`, and `annorex rex open last`.                                                                             |
| Review             | Planned | Keeps annotations on character ranges. Saves drafts after each change. Exports the notes as Markdown and sends them.                                                                 |

## Crates

- `annorex-hosts` (built): the adapted transcript readers. This crate is separate, so the license boundary is clear.
- `annorex` (built): a library and a binary for all other parts. The Lua module is in `crates/annorex/lua/`.

## Key constraints

- Rust 1.99.1, edition 2024, ratatui 0.30.2, crossterm 0.29.0. The Lua version in Rex is 5.1.
- A Rex custom action must finish in 30 seconds. Thus, the Lua module only opens the pane.
- A pane that Rex creates gets its data from command-line arguments. It does not get environment variables from the
  layout.
