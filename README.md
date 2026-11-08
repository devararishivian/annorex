# Annorex

Annorex is a review tool for coding agents in the [Rex](https://www.superlogical.com/rex) terminal. You annotate the
text that an agent wrote. Then Annorex sends your notes back to the agent.

> **Status: design stage.** No usable code exists yet. This README describes the planned features. The sections change
> when features are complete.

## Planned features

- **Review the last reply.** Annorex opens in a pane next to the agent and shows the agent's last reply. You can also
  select an earlier reply.
- **Review files.** You can open one or more files that the agent wrote. Annorex sends all notes in one message.
- **Annotate exact text.** Select the words. Then add a comment, mark the text for removal, or give a replacement.
- **Send safely.** Annorex sends only when the agent is idle. It does not send when the agent is busy or shows a
  permission prompt. It confirms that the message arrived.
- **Keep your notes.** Annorex saves your notes after each change. A refused send does not delete them.
- **Fit any pane size.** The interface changes its layout when you resize or move Rex panes.

Planned agents: **Claude Code**, **Pi**, and **OpenCode**, in Rex on macOS.

Planned limit: for Pi, Annorex monitors only the main agent loop. Annorex does not monitor subagents that Pi extensions
start.

## How Annorex connects to Rex

Rex does not have a plugin system. Rex has custom actions in its Lua configuration file. Annorex will supply:

1. A small Lua module. The module adds two actions to the Rex command palette: "review the last reply" and "review
   files".
2. A Rust program. The program runs in the pane that the actions open.

For more information, see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Planned requirements

- Rex on macOS (Apple silicon)
- Claude Code, Pi 1.5.0 or later, or OpenCode 2.x
- Rust (stable), to build from source

## License

MIT.

## Acknowledgements

These projects are the base of the Annorex idea:

- [Plannotator](https://github.com/backnotprop/plannotator) by backnotprop. Plannotator started the "annotate, then send
  back to the agent" review method.
- [herdr-annotate](https://github.com/plannotator/herdr-annotate) and
  [plannotator-tui](https://github.com/plannotator/plannotator-tui) by Plannotator and Michael Ramos. These projects do
  the same review in a terminal. Annorex will use the transcript readers from plannotator-tui under the MIT License.
  Annorex will keep their copyright notices.
- [memex](https://github.com/nicosuave/memex) by Nico Ritschel. memex shows how to connect a Rust tool to Rex.

Annorex is an independent project. It has no connection to these projects, and they do not endorse it.
