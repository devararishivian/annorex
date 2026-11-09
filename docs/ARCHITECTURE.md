# Architecture

> **Current state: designed, not built.** All parts in this document are **planned**. No code exists yet. This document
> changes when each part is complete.

## Overview

```
Rex init.lua ── require("annorex").setup{} ──▶ annorex.lua           (planned)
                                                  │ custom actions: annorex_last, annorex_files
                                                  ▼ opens a split next to the agent pane
                                     ┌──────────────────────────────────┐
                                     │ annorex (Rust binary)  (planned) │──▶ rex CLI (JSON on stdin)
                                     └──────────────────────────────────┘──▶ agent transcripts (read-only)
```

Annorex has two parts:

1. **Lua module** (`annorex.lua`, planned). Rex does not have a plugin system. Rex has custom actions in its Lua
   configuration file. The module registers the actions. Each action opens Annorex in a split next to the focused agent
   pane. The action gives the pane ID to Annorex as an argument. The module contains no other logic.
2. **Rust binary** (`annorex`, planned). The binary does all other work. It uses only the `rex` command-line tool to
   communicate with Rex. It sends the arguments as JSON on standard input. It never opens a network port.

## Planned components

| Component          | Function                                                                                                                                             |
|--------------------|------------------------------------------------------------------------------------------------------------------------------------------------------|
| Rex adapter        | Runs `rex` commands. Gives typed results for the process, program status, terminal modes, and size of a pane.                                        |
| Agent detection    | Identifies Claude Code, Pi, or OpenCode from the program status of the pane. If no status is available, uses the process name.                       |
| Dialog check       | Refuses a send when the program status of the agent is "blocked" (a permission, question, or authentication dialog). A busy agent can get a message. |
| Payload            | Removes escape characters and control characters. Puts the text in one bracketed paste.                                                              |
| Delivery           | Checks for a dialog again, pastes the text, and sends Enter. It never sends again automatically.                                                     |
| Transcript readers | Find the session of an agent and read its recent replies. Adapted from plannotator-tui (MIT).                                                        |
| Document model     | Shows Markdown or plain text. Keeps a map from each displayed line to its source line and character position.                                        |
| Review             | Keeps annotations on character ranges. Saves drafts after each change. Exports the notes as Markdown.                                                |
| UI                 | ratatui screens. The layout is calculated again after each resize.                                                                                   |

## Planned crates

- `annorex-hosts`: the adapted transcript readers. This crate is separate, so the license boundary is clear.
- `annorex`: a library and a binary for all other parts.

## Key constraints

- Rust 1.99.1, edition 2024, ratatui 0.30.2, crossterm 0.29.0. The Lua version in Rex is 5.1.
- A Rex custom action must finish in 30 seconds. Thus, the Lua module only opens the pane.
- A pane that Rex creates gets its data from command-line arguments. It does not get environment variables from the
  layout.
