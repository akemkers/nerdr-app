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

## [2026-11-19] fix | active pane frame was one key late
gpui: notify inside a focus listener doesn't bring a frame; defer it.

## [2026-11-24] change | spaces became tasks with an origin (#19)
tasks.rs holds the model without GPUI; the old layout shape is ignored, not converted, on the user's wish.

## [2026-11-25] add | new task from a repo (#22)
prefix+f: repo popup over NERDR_MAPPER, used repos in ~/.local/state/nerdr/repos, claude as the first pane.

## [2026-11-25] fix | cell backgrounds covered block elements (#27)
Backgrounds are painted before other rects; the Claude Code mascot body was a black square.

## [2026-11-25] add | new task in its own worktree (#23)
Name step in the prefix+f popup; worktree.rs wraps git (fetch, worktree add from origin/main or an existing branch) under NERDR_WORKTREES.

## [2026-11-25] add | agent status from hc, done until seen (#24)
agent: the status is hc's OSC 7501, done stays green until seen. Hooks via `--settings`, "Agent mangler" and restart were built, then dropped on the user's wish; what they taught is kept. hc: labels events and the terminal's environment. testing: a real Claude in a test session.

## [2026-11-25] add | diff overlay on prefix+d (#21)
jetdiff v0.3.0 as a git dep; one RepoView per task, its keys under jetdiff's key context, Quit leaves it. First open is on the main thread since RepoSource isn't Send.

## [2026-11-25] add | sidebar as task cards, tabs along the top (#25)
sidebar: cards with repo and branch read from git every 2 s, the task's most urgent status and its age from hc's status_ms (missing from status events). Tabs moved to a bar over the panes.
