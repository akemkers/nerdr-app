---
sources: [commits ad6910a 57f03e4 2712d18 28d7583 a040014 c0bb232, crates/nerdr/src/keys.rs, crates/nerdr/src/ime.rs, crates/nerdr/src/view.rs]
updated: 2026-11-19
---
# Input in nerdr

How keyboard and mouse input reaches the program, and why it is built this way. GPUI's side of it is in [[gpui]].

## Keys

- Two paths. Plain text keys (no ctrl, cmd or fn; on Linux no alt either) are left unstopped in `on_key_down` and arrive as text through the input handler in `ime.rs`, so dead keys and IMEs can compose. Everything else is encoded in `keys.rs`. Why: GPUI only lets the IME see keys the key handler didn't stop. Commit 2712d18.
- Exception: when the program asked for kitty's "all keys as escape codes" (flag 8), text keys go through `keys.rs` too, without composing, because the program wants key events, not text.
- `keys.rs` encodes legacy xterm style, or the kitty protocol when the program pushed kitty flags (alacritty tracks them with `kitty_keyboard: true`). All five flags; lone modifiers and the base layout key are missing because GPUI gives neither. Commit 57f03e4. Protocol rules: [[kitty]].
- Releases go out only for keys whose press went out (`pressed` set in `TerminalView`). Why: the workspace and the copy/paste keys take presses the program never saw, and their releases would otherwise leak. Commit 57f03e4.
- Holding a key repeats it. nerdr's own `InputHandler` turns off macOS press-and-hold (the accent menu). Commit 2712d18.

## Option and alt

- On macOS option types characters when the layout gives one (`[]{}|@` on Norwegian layouts), the same way shift does. Without one (option+arrow) it is sent as alt. Why: those characters can't be typed otherwise. Commit 28d7583.
- Cost: option is no meta key on macOS, so alt+b / alt+f in the shell type characters. Rejected: kitty's "right option is alt", because GPUI's `Modifiers` has no left/right. Open: whether the user needs meta.
- Tab jumping is cmd+1…9 on macOS and alt+1…9 on Linux. Why: alt+digit is option+digit on macOS and stole `[` `]`. Commit a040014. The other workspace keys are still ctrl+shift on both; moving them to cmd on macOS is an open question.

## Mouse and clipboard

- No primary selection: selecting doesn't copy and middle-click doesn't paste. Copy and paste are ctrl+shift+c/v, the same on every platform. The user's choice; also, GPUI only has the primary selection on Linux. Commit ad6910a.
- Middle-clicks may never reach nerdr on Linux (GPUI didn't call the handler when tested), so mouse-reporting programs may get none. Not investigated. `docs/spike.md`.
- OSC 8 links: hovering underlines the link and shows a hand cursor; a click without a drag opens it, like kitty. A drag still selects. Commit c0bb232. Links in the attach paint are lost: [[hc]].
