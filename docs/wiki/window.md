---
sources: [commits 2ce9a62, crates/nerdr/src/main.rs, crates/nerdr/src/view.rs, crates/nerdr/src/workspace.rs]
updated: 2026-11-19
---
# The nerdr window

- Opens maximized: it fills the screen and keeps the menu bar and Dock. Rejected: native fullscreen (`WindowBounds::Fullscreen`), its own Space on macOS, which the user didn't want. Commit 2ce9a62.
- The window title is fixed: `nerdr-app v<version>` from `Cargo.toml`, plus `[reconnecting…]` or the exit reason when the stream is down. That suffix is the only place this state shows. Commit 2ce9a62.
- Why fixed: shells set the title on every command, and a short command made it flash ([[shell-titles]]). Rejected: showing program titles only after they've lasted a moment (a timer deciding what you see), and showing program titles at all, since the sidebar already names the tabs.
- Program titles (OSC 0/2) name the tab in the sidebar instead.
- The title is set only when it changes, through a value shared by all tabs. Why: `render` runs on every output batch, and setting the title on macOS is not free ([[gpui]]).
