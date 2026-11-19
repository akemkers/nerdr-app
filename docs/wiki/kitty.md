---
sources: [~/.config/kitty/kitty.conf, ~/.config/kitty/current-theme.conf, https://sw.kovidgoyal.net/kitty/keyboard-protocol/, commits 2ce9a62 57f03e4]
updated: 2026-11-19
---
# kitty

## The look nerdr copies

- The user's kitty: `Hack Nerd Font Mono` 16, `modify_font cell_height 130%`, Catppuccin Mocha with background `#181825`. nerdr matches it, commit 2ce9a62.
- Kitty rounds the font's line height up to whole pixels before the 130%: 18.6 px → 19 → 25 px rows, which the screenshot showed. Inferred from the screenshot, not from kitty's source.
- Kitty draws box drawing (U+2500–257F) itself so lines fill the cell. With 130% line height, font glyphs leave gaps between rows. nerdr draws them too, commit 2ce9a62.

## Keyboard protocol

- Flag 1 (disambiguate): Esc, and keys with ctrl/alt, go out as `CSI code;mods u`. Enter, Tab, Backspace stay legacy unless modified, so shift+enter is `CSI 13;2u`. Text keys (plain or shift) still send text.
- Flag 2 (event types): `:2` repeat, `:3` release, in the modifier field. Text keys get no events, and Enter/Tab/Backspace no releases, unless flag 8.
- Flag 8 (all keys as escape codes): text keys too, and no text is sent. Flag 16 adds the text as a third field, with an empty modifier field when it's 1.
- F3 is `CSI 13 ~` (legacy `CSI R` collides with the cursor position report).
- Before nerdr spoke the protocol, the user's kitty had `map shift+enter send_text all \e\r` for Claude Code.
