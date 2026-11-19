# nerdr-app

A native desktop app for terminals and coding agents, written in Rust with GPUI. herdr-core (`hc`) is the backend: it runs every terminal and agent; nerdr-app is a client of it, like the Herdr app.

## Constraints

- GPUI is pinned to the same version as `~/git/jetdiff` (`gpui = "0.2.2"`). Don't change it without changing jetdiff too: jetdiff's crates (`jetdiff-ui` and others) will be embedded as a diff pane.
- herdr-core API: JSON-RPC 2.0 over the hc socket, one message per line, or framed after `hello`. The full API as OpenRPC: `hc schema`. Manual: `hc --skill`. Protocol generation 1.
- Linux first (Hyprland, Wayland).

## Wiki

`docs/wiki/` holds facts learned about hc, GPUI, macOS, kitty and the shell. Read `docs/wiki/index.md` before working against one of them. When a task teaches you something durable about them, add it (rules: `docs/wiki/README.md`).

## Status

Spike done, plus the connection layer (`hc::Client`: many streams on one socket, events, reconnect) and spaces/tabs with a sidebar (`workspace.rs`, layout saved in the hc doc `nerdr`), and a hardened terminal view (kitty keyboard, IME, OSC 8 links, cursor blink). Runs on macOS too. See docs/spike.md.
