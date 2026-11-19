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
- superseded 2026-11-19: tab jumping was cmd+1…9 on macOS and alt+1…9 on Linux (commit a040014), because alt+digit stole option+digit's `[` `]`. Replaced by tmux-style prefix keys, see below.

## Workspace keys: tmux-style

- The user is a tmux user and wants to recognise the controls at once, so the workspace keys are tmux's: a prefix, then tmux's key for the command. The model is the user's tmux nerdr (`~/git/nerdr`, README "Prefikset"). Spaces are tmux's sessions, tabs its windows, splits will be its panes.
- Why a prefix and no modifier shortcuts: nothing is taken from the program in the terminal, and option/alt/cmd stay free on every platform (the macOS collisions above are gone).
- A key after the prefix is matched by the character it types (`key_char`), as tmux names keys, so `&` `(` `)` work on Norwegian layouts where they are shift+digit. `prefix.rs`.
- The prefix pressed twice sends it to the program, like tmux's `send-prefix`.
- The prefix, `base-index` and the bindings come from the running tmux server (`show-options -gv prefix`, `show-options -gv base-index`, `list-keys -T prefix`), as the tmux nerdr does, so the user's own bindings work (`X` closes a tab, the unbound `&` does nothing). kill-pane (`x`) also closes the tab while a tab is one pane, as in tmux when the last pane goes. tmux commands map to actions in `prefix.rs` (`command_action`); `f` is always nerdr's new space. `NERDR_PREFIKS` overrides the prefix, the same variable as in the tmux nerdr so one setting covers both.
- Rejected: starting a tmux server to read `tmux.conf` when none runs, because that runs the user's plugins too (tpm, continuum's session restore). Without a running tmux the defaults apply. Also rejected: parsing `tmux.conf` ourselves, since tmux's own parser is the only exact one.
- `NERDR_DEBUG=1` logs the prefix, base-index and the mapped bindings at start.
- Splits are tmux's: `split-window -h` side by side, `-v` (or no flag) one above the other, `kill-pane` closes the pane and the tab with its last pane. A new pane always starts in the active pane's directory, as with the user's `-c "#{pane_current_path}"`; tmux without `-c` would use the session's directory, which nerdr doesn't have.
- Moving between panes is tmux's select-pane `-L/-R/-U/-D` (arrows, `C-h/j/k/l`) and resize-pane -Z (`z`). Anything that changes the active pane ends a zoom. Which pane is tmux's rule (`window_pane_choose_best`): of the panes touching that edge, the most recently active one, so going down and back up returns to where you were. At the tab's edge nothing happens: tmux wraps to the opposite side, but the user found that confusing and asked for a no-op (2026-11-19). nerdr first picked the touching pane with the most overlap; with two equal ones above, `C-k` after `C-j` went to the left one instead of back, which felt wrong. Ties go to most overlap. `panes.rs` (`neighbor`), `Tab.recent`.
- `o` and `;` (next and last pane) are left out on purpose: the user doesn't want them, so nerdr has no action for `select-pane -t :.+` or `last-pane` even though tmux binds them.
- `C-h/j/k/l` come from vim-tmux-navigator's root bindings, `if-shell "ps … | grep -iqE '<pattern>'" "send-keys C-h" "select-pane -L"`. nerdr reads the pattern from the binding and matches it against hc's foreground program of the active pane (`"S <name>"`, standing in for a `ps -o state= -o comm=` line): a match (nvim, fzf …) gets the key, else the pane changes. The plugin's check covers every process on the tty; the foreground program is close enough for vim and fzf. `prefix.rs` (`vim_navigator`).
- tmux command lines are split as tmux quotes them (`"…"` with escapes, `'…'`, `\` outside quotes): `tokens` in `prefix.rs`.
- tmux's `-r` works: after a repeatable key, more of them work without the prefix for `repeat-time` (500 ms), read from tmux. Any other key ends it and goes to the program, as in tmux.
- tmux's root table (`bind -n`) works too: the user's `M-arrows` resize without the prefix. Root bindings nerdr can't run (other `if-shell`s, mouse bindings) are left out, so those keys reach the program.
- tmux 3.8 writes its defaults for `C-`/`M-Up` and `Left` as `if-shell -F "#{?floating_pane_flag}" { floating } { tiled }`. nerdr has no floating panes, so it reads the second block. Without this only Down and Right resized.
- Copy and paste stay ctrl+shift+c/v for now; tmux's copy mode (`[`) isn't built.

## Mouse and clipboard

- No primary selection: selecting doesn't copy and middle-click doesn't paste. Copy and paste are ctrl+shift+c/v, the same on every platform. The user's choice; also, GPUI only has the primary selection on Linux. Commit ad6910a.
- Middle-clicks may never reach nerdr on Linux (GPUI didn't call the handler when tested), so mouse-reporting programs may get none. Not investigated. `docs/spike.md`.
- OSC 8 links: hovering underlines the link and shows a hand cursor; a click without a drag opens it, like kitty. A drag still selects. Commit c0bb232. Links in the attach paint are lost: [[hc]].
