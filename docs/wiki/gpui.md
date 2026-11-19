---
sources: [gpui-0.2.2/src (cargo registry), commits ad6910a 2ce9a62 2712d18 28b5f4a a040014]
updated: 2026-11-19
---
# GPUI 0.2.2

Paths are under `~/.cargo/registry/src/index.crates.io-*/gpui-0.2.2/src/`.

## Keys and text input

- macOS order (`platform/mac/window.rs`, `handle_key_event`): `on_key_down` listeners run first. A key they don't stop goes on to the input handler and the IME. Exception: while composing, or for a key without `key_char` (and no ctrl/fn), the IME gets it first.
- Wayland (`platform/linux/wayland/window.rs`, `handle_input`): an unhandled key goes to the input handler only when its modifiers are a subset of shift.
- So a terminal leaves plain text keys unstopped and takes text from `InputHandler::replace_text_in_range`. nerdr: `ime.rs`, commit 2712d18.
- `InputHandler::apple_press_and_hold_enabled` defaults to `true` (`platform.rs`): holding a key opens the accent menu instead of repeating. `ElementInputHandler` doesn't let you change it; implement `InputHandler` yourself and return `false`.
- `Modifiers` has no left/right side, so kitty's `macos_option_as_alt right` can't be copied. On macOS, option+digit is a text key on Norwegian layouts and collides with alt+digit shortcuts. Commits 28d7583, a040014.
- `KeyDownEvent::is_held` marks repeats; `KeyUpEvent` exists for releases.

## Window

- `read_from_primary` / `write_to_primary` exist only on Linux and FreeBSD (`app.rs`, `#[cfg]`). Code calling them doesn't build on macOS. Commit ad6910a.
- `Window::window_title()` is macOS only; other platforms return `""` (`platform.rs`, `get_title` default).
- On macOS `set_window_title` sets the NSWindow title, updates the Window menu and moves the traffic lights on every call (`platform/mac/window.rs`, `set_title`). Calling it from `render` does that on every output batch; call it only on change.
- `WindowBounds::Fullscreen` is native macOS fullscreen (its own Space); `Maximized` fills the screen and keeps the menu bar.
- A scale change (another display) calls the resize callback, so `observe_window_bounds` observers fire (`window.rs`, `bounds_changed`). Commit 28b5f4a.

## Fonts and drawing

- `WindowTextSystem::font_id` is private. Check that a font is installed with `all_font_names()`; `resolve_font` silently falls back. Commit 2ce9a62.
- `PathBuilder::stroke(width)` with `move_to`/`line_to`/`curve_to` and `window.paint_path` draws curves (rounded box corners). Commit 2ce9a62.
- `layout_line` merges neighbouring runs with the same color even when the fonts differ, so bold in the default color came out regular. nerdr works around it by starting a new segment at every font change. `docs/spike.md`. Not reported upstream yet; worth an issue (jetdiff may hit it too).
- `gpui::deferred(el)` paints after the rest of the window with an empty clip stack (`window.rs`, `paint_deferred_draws`), so it can draw past an `overflow_hidden` parent and on top of later siblings. Its layout and position stay where it sits in the tree. nerdr's active-pane frame uses it to cover the 1 px lines around the pane.
