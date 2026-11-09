# Architecture

> **Current state: foundation built.** The Rex adapter, agent detection, dialog check, payload, delivery, transcript
> readers, and the `probe` and `deliver` commands are **built**. The Lua module, document model, review, and UI are
> **planned**.

## Overview

```
Rex init.lua ── require("annorex").setup{} ──▶ annorex.lua           (planned)
                                                  │ custom actions: annorex_last, annorex_files
                                                  ▼ opens a split next to the agent pane
                                     ┌──────────────────────────────────┐
                                     │ annorex (Rust binary)            │──▶ rex CLI (JSON on stdin)
                                     └──────────────────────────────────┘──▶ agent transcripts (read-only)
```

Annorex has two parts:

1. **Lua module** (`annorex.lua`, planned). Rex does not have a plugin system. Rex has custom actions in its Lua
   configuration file. The module registers the actions. Each action opens Annorex in a split next to the focused agent
   pane. The action gives the pane ID to Annorex as an argument. The module contains no other logic.
2. **Rust binary** (`annorex`, built: `probe` and `deliver`). The binary does all other work. It uses only the `rex`
   command-line tool to communicate with Rex. It sends the arguments as JSON on standard input. It never opens a
   network port.

## Components

| Component          | State   | Function                                                                                                                                                                             |
|--------------------|---------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Rex adapter        | Built   | Runs `rex block call -s SESSION -b BLOCK METHOD -` with the arguments on standard input. Gives typed results for the process, program status, and terminal modes of a pane.          |
| Agent detection    | Built   | Identifies Claude Code, Pi, or OpenCode from the `app` of the root program status record, when the foreground process confirms it. Otherwise uses `argv0` of the foreground process. |
| Dialog check       | Built   | Refuses a send when the root program status is "blocked" (a permission, question, or authentication dialog). A busy agent can get a message.                                         |
| Payload            | Built   | Removes escape characters and control characters. Keeps line feeds and tabs. Puts the text in one bracketed paste.                                                                   |
| Delivery           | Built   | Checks for a dialog two times, pastes the text, waits 300 ms, checks again, and sends Enter. It never sends again automatically.                                                     |
| Transcript readers | Built   | Find the session of an agent and read its recent messages. Adapted from plannotator-tui (MIT).                                                                                       |
| Commands           | Built   | `annorex probe` and `annorex deliver`.                                                                                                                                               |
| Document model     | Planned | Shows Markdown or plain text. Keeps a map from each displayed line to its source line and character position.                                                                        |
| Review             | Planned | Keeps annotations on character ranges. Saves drafts after each change. Exports the notes as Markdown.                                                                                |
| UI                 | Planned | ratatui screens. The layout is calculated again after each resize.                                                                                                                   |

## Crates

- `annorex-hosts` (built): the adapted transcript readers. This crate is separate, so the license boundary is clear.
- `annorex` (built in part): a library and a binary for all other parts.

## Key constraints

- Rust 1.99.1, edition 2024, ratatui 0.30.2, crossterm 0.29.0. The Lua version in Rex is 5.1.
- A Rex custom action must finish in 30 seconds. Thus, the Lua module only opens the pane.
- A pane that Rex creates gets its data from command-line arguments. It does not get environment variables from the
  layout.
