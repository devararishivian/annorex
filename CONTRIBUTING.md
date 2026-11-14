# Contributing to Annorex

Annorex lets you annotate the replies and files of a coding agent in a Rex pane. Thank you for helping. This guide
lists what you need, the commands to run, and the steps for a pull request.

Everyone who takes part must follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## What you need

- Rust 1.99.1 or newer. The workspace sets `rust-version = "1.99"`, and CI builds with 1.99.1. The
  workspace uses the 2024 edition.
- macOS. The release targets are `aarch64-apple-darwin` and `x86_64-apple-darwin`.
- The `rex` CLI from the Rex terminal. Install Rex from its own site.
- Optional, for the live tests only: Claude Code, Pi, and OpenCode.

The workspace has two crates: `crates/annorex` (the program) and `crates/annorex-hosts` (the readers for the agents).

## Build

1. Clone the repository and open its root directory.
2. Run `cargo build --workspace`.

## Run the CI commands

CI runs these commands on every push and pull request. Run the same commands before you open a pull request.

1. Format: `cargo fmt --all -- --check`
2. Lint: `cargo clippy --workspace --all-targets -- -D warnings`
3. Test: `cargo test --workspace`
4. Licenses: `cargo deny check licenses sources bans`

The fourth command needs `cargo-deny`. CI uses version 0.20.2. Install it with `cargo install --locked cargo-deny --version 0.20.2`.

The workspace forbids `unsafe` code. Clippy runs with the `pedantic` lints, and `unwrap`, `expect`, `panic`, and
`print_stdout` produce warnings. CI turns each warning into an error.

## Live tests

The live tests are in `crates/annorex/tests/live_*.rs`. They are marked `#[ignore]`, so `cargo test --workspace` skips
them. They run real agents in a temporary Rex session.

Before you run a live test, make sure of these points:

1. Rex runs on your machine and the `rex` CLI works.
2. The agents that the test uses are installed. Some tests also make model calls. Read the comment at the top of the test file.
3. The environment variable `ANNOREX_LIVE=1` is set.

Run one live test with this command. Replace `live_layer` with the name of the test file.

```text
ANNOREX_LIVE=1 cargo test -p annorex --test live_layer -- --ignored --nocapture
```

Each test removes its Rex session and its temporary directories, also when it fails.

## Commit messages

Use conventional commits. Write the type, an optional scope in brackets, a colon, and a short summary in lowercase.

```text
fix(drafts): prune keeps empty drafts and removes old-name copies
feat(drafts): clear sent notes and keep work in progress
docs(architecture): describe the batch redraw and the strip position
```

The types in the history are `feat`, `fix`, `docs`, `test`, `perf`, `ci`, and `build`.

## Pull requests

1. Branch from `develop` and open the pull request against `develop`.
2. Add or change tests for each change in behavior.
3. Run the four commands in "Run the CI commands". All must pass.
4. Update the documentation that your change affects.
5. Fill in the pull request template.

Keep each pull request to one change. Explain what the change does and why.

## Reports

To report a bug or ask for a feature, open an issue and use the form. To report a security problem, follow
[SECURITY.md](SECURITY.md). Do not open a public issue for it.
