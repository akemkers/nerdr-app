---
sources: [herdr-core crates/ghostty-vt/src/lib.rs (paint) and crates/herdr-core/src/viewers.rs at 329cb66, https://github.com/herdrdev/herdr-core, commit 6929829, commit 28b5f4a, commit c0bb232, hc schema]
updated: 2026-11-26
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
- The paint breaks soft-wrapped rows with `\r\n`, as hard lines at the attach width: `paint` in herdr-core `crates/ghostty-vt/src/lib.rs` formats with `unwrap: false`. Seen in a trace (`NERDR_TRACE`), hc 0.1.21; unchanged in 0.1.22 and main on 2026-11-26.
- So a viewer can't reflow the paint as the core does. After a resize the client's lines broke as 71 + 9 (the paint was at 80), while the core's were right; Claude drawing over that left letters everywhere. The user saw it after unplugging an external display, 2026-11-26.
- A resize by a typing viewer gets no new paint. Replacement paints after a resize go only to observers that ask for them (`observe` and `resize_notifications`, `apply_size` in herdr-core `viewers.rs`), and carry no history (`first_paint(st, id, 0)`). An observer can't type or resize: the size comes from the most recently active typing viewer, else the headless 200x120.
- The newest typing viewer leads the size: `terminal.attach` sets its `active_at` and applies its size before making the paint. Closing a viewer re-applies the size from the ones left.
- nerdr's answer: every resize attaches again for a fresh paint at the new size (`Stream::resize` in `hc.rs`), and the old channel closes once the new one is up, so the terminal never falls back to its headless size. An attach still out when the size changes again is asked again on its reply. Rejected: a second, observer stream per pane (output twice, history stays wrong); waiting for a herdr-core change. Measured: growing the window at start is ~30 resizes and as many paints, 5.4 MB in all with 5000 lines of colored history, so no debounce. 2026-11-26.
- The core answers kitty's `CSI ? u` itself and restores the kitty flags in the paint (`CSI = flags ; 1 u`). The client only tracks the flags. `docs/spike.md`.
