---
sources: [https://github.com/herdrdev/herdr-core, commit 6929829, commit 28b5f4a, commit c0bb232, hc schema]
updated: 2026-11-25
---
# hc (herdr-core)

## Install

- `hc` is herdr-core, a separate program from `herdr` (the Herdr TUI). Install: `curl -fsSL https://releases.herdr.dev/core/install.sh | sh`, lands in `~/.local/bin/hc`. Source: herdr-core README.

## Socket path

- Order, as in herdr-core `crates/herdr-core/src/paths.rs` and `platform/unix.rs`: `$HC_SOCKET`, else `socket` in `~/.config/herdr-core/config.toml`, else `$XDG_RUNTIME_DIR/herdr-core/<session>.sock` (or `/run/user/<uid>/herdr-core/` if that exists), else beside the state: `$XDG_STATE_HOME/herdr-core/<session>/<session>.sock`, `~/.local/state` by default. Session is `HC_SESSION` trimmed, `default` when empty.
- On macOS there's no `XDG_RUNTIME_DIR`, so the socket is `~/.local/state/herdr-core/default/default.sock`. Observed 2026-11-18.
- nerdr follows the order except the config `socket` and the `/run/user` fallback. Commit 6929829.

## Testing

- Isolated test sessions (`HC_SESSION`), `hc run --bg --json`, and the risks of `hc send`: [[testing]].

## Labels and a terminal's environment

- A terminal's program gets `HC_TERMINAL_ID` (its address), `HC_SESSION` and `HC_TERMINAL_SOCKET`, so `hc label "$HC_TERMINAL_ID" k=v` inside it reaches its own core, also in a test session. `hc` is on its PATH (`~/.local/bin`). Observed with hc 0.1.21.
- `hc label t k=v` / `terminal.label` sends an event `{"type": "labels", "data": {"labels": {...}}}` with all the labels as they are now; `k=` removes one. The `Terminal` record has them under `labels`. Observed with hc 0.1.21, `hc schema`.
- `terminal.run` takes `labels`; `hc run --label k=v` (repeatable) too.
- The core's environment goes to every terminal. A core started from inside a Claude session passes `CLAUDE_CODE_CHILD_SESSION` and the like on, and a Claude in it says "Transcript saving is off", so `--continue` finds nothing later. Observed 2026-11-25; see [[testing]].

## Attach, paint, display

- `display` (colors and cell size) is only taken in `terminal.attach`; no call updates it later. hc 0.1.21 `hc schema`. nerdr attaches again when the scale changes, commit 28b5f4a.
- The paint on attach doesn't carry OSC 8: a link printed before the attach arrives as plain text. Live DATA passes OSC 8 through. Observed with hc 0.1.21, commit c0bb232. Not reported to herdr-core yet; worth an issue.
- The core answers kitty's `CSI ? u` itself and restores the kitty flags in the paint (`CSI = flags ; 1 u`). The client only tracks the flags. `docs/spike.md`.
