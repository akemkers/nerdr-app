# Log

## [2026-11-19] add | First pages from the macOS bring-up and hardening
Pages: hc, gpui, macos, kitty, shell-titles. From the sessions behind commits ad6910a..a040014.

## [2026-11-19] add | nerdr's own decisions
Pages: input, rendering, window. Decisions from commits ad6910a..a040014, with what was rejected.

## [2026-11-19] add | Testing nerdr as an agent
New page testing; hc's test section moved there.

## [2026-11-19] add | Daily use and upstream reports
New page daily-use. hc and gpui mark the OSC 8 paint gap and the layout_line bug as worth reporting.

## [2026-11-19] add | tmux-style workspace keys
input: prefix keys replace the modifier shortcuts (cmd/alt+digit superseded). daily-use: the user is a tmux user.

## [2026-11-19] add | Keys from tmux
input: prefix and bindings read from the running tmux, and why no tmux server is started for it.

## [2026-11-19] add | Splits
input: split and kill-pane as in tmux.

## [2026-11-19] add | Resizing panes
input: repeat keys, the root table, and tmux 3.8's floating-pane if-shell defaults.

## [2026-11-19] add | C-hjkl and no o/;
input: vim-tmux-navigator read from the root table; o and ; dropped on the user's wish.

## [2026-11-19] fix | active pane frame
gpui: deferred escapes clipping and paints on top. Lines no longer light up per split, which lit every ancestor split.

## [2026-11-19] fix | select-pane picks the most recent pane
input: tmux's rule for which touching pane select-pane goes to.

## [2026-11-19] decision | no wrapping at the edges
input: select-pane does nothing at the tab's edge, on the user's wish.
