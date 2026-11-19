---
sources: [https://github.com/herdrdev/herdr-core, commit 6929829, commit 28b5f4a, commit c0bb232, hc schema]
updated: 2026-11-19
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

## Attach, paint, display

- `display` (colors and cell size) is only taken in `terminal.attach`; no call updates it later. hc 0.1.21 `hc schema`. nerdr attaches again when the scale changes, commit 28b5f4a.
- The paint on attach doesn't carry OSC 8: a link printed before the attach arrives as plain text. Live DATA passes OSC 8 through. Observed with hc 0.1.21, commit c0bb232. Not reported to herdr-core yet; worth an issue.
- The core answers kitty's `CSI ? u` itself and restores the kitty flags in the paint (`CSI = flags ; 1 u`). The client only tracks the flags. `docs/spike.md`.
