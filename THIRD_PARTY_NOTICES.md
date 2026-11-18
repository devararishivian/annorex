# Third-party notices

Annorex contains code from the projects below. Each vendored Rust file starts with a header that gives its origin and
the changes for Annorex. A comment marks the adapted functions in `wrap.rs` and `editor.rs` and the lint table in
`Cargo.toml`. The test fixtures and `rustfmt.toml` have no header. This file lists them.

## plannotator-tui

- Project: plannotator-tui
- URL: <https://github.com/plannotator/plannotator-tui>
- Commit: `83b0cd91e287df391d5b27029140ff72c32c085d`, for all parts except the keys of the note box, the skipped
  folders, the hidden toggle, and the hanging indent. These parts come from the later commit
  `e537abdefa7f404c62bb51f927cbe426fbdf500c`. The license did not change.
- License: MIT
- Files in Annorex that come from plannotator-tui:
    - `crates/annorex-hosts/src/lib.rs`, `claude/mod.rs`, `claude/entries.rs`, `claude/ladder.rs`,
      `claude/messages.rs`, `opencode.rs`, `pi.rs`, `sqlite.rs`, and `time.rs` (from
      `crates/plannotator-tui-hosts/src/`)
    - `crates/annorex-hosts/src/roots.rs` (from `crates/plannotator-tui/src/last/roots.rs`)
    - `crates/annorex-hosts/tests/claude.rs`, `exact_session.rs`, `ladder.rs`, `opencode.rs`, `pi.rs`, and `roots.rs`
      (from `crates/plannotator-tui-hosts/tests/`)
    - The test fixtures in `crates/annorex-hosts/tests/fixtures/` (from `crates/plannotator-tui-hosts/tests/`), except
      `claude-code-new-records.jsonl` and `pi-new-records.jsonl`
    - The function `wrap_table` in `crates/annorex/src/wrap.rs`: the column widths and the clip fallback
    - The keys and the word rules of the note box in `crates/annorex/src/ui/editor.rs` (from
      `crates/plannotator-tui/src/ui/compose.rs` at commit `e537abd`)
    - The list of skipped folders (`SKIPPED_DIRS`) and the toggle for hidden files in `crates/annorex/src/files.rs`
      and the tree (from `crates/plannotator-tui/src/tree.rs` at commit `e537abd`)
    - The hanging indent of wrapped list items and quotes, with its half-width rule, in `crates/annorex/src/wrap.rs`
      (from `crates/plannotator-tui/src/wrap/hang.rs` at commit `e537abd`)
    - The lint table in the root `Cargo.toml` and the root `rustfmt.toml`
- These files in `crates/annorex-hosts/` are Annorex code: `changes.rs`, `claude/changes.rs`, `pi_changes.rs`,
  `opencode_changes.rs`, `tests/display_name.rs`, `tests/fixtures/claude-code-new-records.jsonl`, and
  `tests/fixtures/pi-new-records.jsonl`.

```text
MIT License

Copyright (c) 2026 Michael Ramos

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
