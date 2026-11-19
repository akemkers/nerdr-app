---
sources: [commits 2ce9a62 d8dca33 c0bb232 2712d18, crates/nerdr/src/view.rs, crates/nerdr/src/theme.rs]
updated: 2026-11-19
---
# Rendering in nerdr

How the terminal is drawn and why it looks the way it does. The reference look is the user's kitty: [[kitty]].

## Font and cell size

- Hack Nerd Font Mono when installed, else the bundled JetBrains Mono. Rejected: bundling Hack, its four Nerd Font files are about 11 MB against about 1 MB for JetBrains Mono. Commit 2ce9a62.
- Size 16 (`NERDR_FONT_SIZE` overrides). Cell width is the advance of `m`; cell height is the font's line height rounded up to whole pixels, times 1.3, as kitty does with `cell_height 130%`. Both snapped to device pixels. Commit 2ce9a62.

## Colors

- Catppuccin Mocha, copied from the user's kitty theme, hardcoded in `theme.rs`. The sidebar colors aren't in kitty's config and were picked from the Mocha palette. Selection is light with dark text, as in kitty. Commit 2ce9a62.
- Making theme and font configurable is in the plan (config and packaging), not done.

## Box drawing

- Light and heavy lines, corners, tees and crosses (U+2500–257F) are drawn as rectangles; `╭╮╯╰` as curves with `PathBuilder`. Why: with 130% line height the font's glyphs stop short of the cell, and vertical lines got gaps. Kitty draws them itself for the same reason. Commit 2ce9a62.
- Double lines (`═║╔`), dashed lines and diagonals still come from the font, so double lines can still have gaps.
- Block elements (U+2580–259F) were already rectangles, for the same reason.

## Cursor and overlays

- The cursor blinks (530 ms) only when the program asks for it and the terminal has focus; a key press shows it again. Commit d8dca33.
- IME composition is drawn underlined at the cursor, painted last so terminal text under it doesn't show through. Commit 2712d18.
- The OSC 8 link under the mouse is underlined. Commit c0bb232.
