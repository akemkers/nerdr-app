# nerdr-app

A native desktop app for terminals and coding agents, written in Rust with GPUI. herdr-core (`hc`) is the backend: it runs every terminal and agent; nerdr-app is a client of it, like the Herdr app.

## Constraints

- GPUI is pinned to the same version as `~/git/jetdiff` (`gpui = "0.2.2"`). Don't change it without changing jetdiff too: jetdiff's crates (`jetdiff-ui` and others) will be embedded as a diff pane.
- herdr-core API: JSON-RPC 2.0 over the hc socket, one message per line, or framed after `hello`. The full API as OpenRPC: `hc schema`. Manual: `hc --skill`. Protocol generation 1.
- Linux first (Hyprland, Wayland).
- Keep the formatting as it is: the code isn't rustfmt-formatted (no `rustfmt.toml`), so `cargo fmt` rewrites files nobody touched. Format only the lines you change.

## Wiki

`docs/wiki/` is the project's memory: how nerdr works and why, decisions, procedures, gotchas, and facts about hc, GPUI, macOS and other tools. Read `docs/wiki/index.md` at the start of a task. When a task teaches you something worth knowing later, add it (rules: `docs/wiki/README.md`).

## Status

Spike done, plus the connection layer (`hc::Client`: many streams on one socket, events, reconnect) and spaces/tabs with a sidebar (`workspace.rs`, layout saved in the hc doc `nerdr`), a hardened terminal view (kitty keyboard, IME, OSC 8 links, cursor blink), and tmux-style keys and splits read from the user's tmux (`prefix.rs`, `panes.rs`). Runs on macOS too. See docs/spike.md.
