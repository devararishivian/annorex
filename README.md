# Annorex

Annorex is a review tool for coding agents in the [Rex](https://www.superlogical.com/rex) terminal. You annotate the
text that an agent wrote. Then Annorex sends your notes back to the agent.

> **Status: foundation complete.** The `annorex probe` and `annorex deliver` commands work. The review interface and
> the Rex actions are not available yet. The "Planned features" section describes them.

## Available now

- `annorex probe` shows the agent in a pane, its state, and whether Annorex can send to it.
- `annorex deliver` sends text from standard input to the agent in a pane. It removes escape characters and control
  characters, pastes the text as one bracketed paste, and presses Enter.

The tests show that a multi-line Markdown message arrives intact in Claude Code, Pi, and OpenCode, also when the agent
is busy.

### Build from source

```sh
cargo build --release
```

The binary is `target/release/annorex`.

### Use

Run the commands in a Rex pane. Give the block ID of the agent pane as `--target`. The session is `$REX_SESSION`, or
give it with `--session`.

```sh
annorex probe --target block:...
annorex probe --target block:... --json
printf 'Fix the test in parser.rs.\n' | annorex deliver --target block:...
printf 'Fix the test in parser.rs.\n' | annorex deliver --target block:... --paste-only
```

`annorex deliver` refuses to send when:

- no supported agent runs in the pane,
- the agent shows a dialog, such as a permission prompt or a question, or
- the pane has not turned on bracketed paste.

When the agent is busy, Claude Code queues the message. Pi and OpenCode use it to steer the current task.

Exit codes:

| Code | Meaning                                                            |
|------|--------------------------------------------------------------------|
| 0    | Sent, pasted, or probe complete                                    |
| 3    | Refused: no agent, a dialog, or no bracketed paste                 |
| 4    | A Rex call failed, or the `rex` binary was not found               |
| 64   | Usage error: a bad argument, no session, or an empty message       |

### Known limit

Claude Code asks whether to trust a folder when it starts in a new folder. Claude Code does not report this dialog to
Rex. Annorex cannot see it. Do not send while this dialog is open.

## Planned features

- **Review the last reply.** Annorex opens in a pane next to the agent and shows the agent's last reply. You can also
  select an earlier reply.
- **Review files.** You can open one or more files that the agent wrote. Annorex sends all notes in one message.
- **Annotate exact text.** Select the words. Then add a comment, mark the text for removal, or give a replacement.
- **Send safely.** Annorex removes escape characters from your notes before it sends them. It does not send when the
  agent shows a dialog, such as a permission prompt. When the agent is busy, Claude Code queues the notes, and Pi and
  OpenCode use them to steer the current task.
- **Keep your notes.** Annorex saves your notes after each change. A refused send does not delete them.
- **Fit any pane size.** The interface changes its layout when you resize or move Rex panes.

Planned agents: **Claude Code**, **Pi**, and **OpenCode**, in Rex on macOS.

## How Annorex connects to Rex

Rex does not have a plugin system. Rex has custom actions in its Lua configuration file. Annorex will supply:

1. A small Lua module. The module adds two actions to the Rex command palette: "review the last reply" and "review
   files".
2. A Rust program. The program runs in the pane that the actions open.

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
