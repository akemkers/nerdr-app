---
sources: [crates/nerdr/src/seen.rs, crates/nerdr/src/workspace.rs (status_color, track_seen), https://code.claude.com/docs/en/hooks, observed 2026-11-25 with Claude Code 2.1.332 and hc 0.1.21]
updated: 2026-11-25
---
# The agent and its status

How nerdr knows what a task's agent is doing. Spec: issue #24 (parent #18).

## hc manages it

- The status is hc's: `status.state` on the terminal (idle, working, done, blocked, error), from the program's OSC 7501 reports (`hc --skill`). nerdr only draws it, as the tab's dot (`status_color`). The user's choice, 2026-11-25: "that's kinda the point with using hc".
- Claude reports it without any setup from nerdr. The user's one status hook, `~/.claude/hooks/herdr-agent-state.sh`, exits unless `HERDR_ENV=1`, so it doesn't run under hc. Seen 2026-11-25.
- It's better than the hooks below: blocked turns back into working once a permission prompt is answered. Observed by the user 2026-11-25. And it works for any Claude in any pane, also one started by hand.
- The agent is plain `claude` in the origin, as before #24.

## Done until seen

- Done is green only until seen (`seen.rs`, kept in nerdr): a pane that gets done while it isn't on screen, or the window isn't active, stays green until it has been on screen with the window active, then grey. On screen: the active tab's panes, or only the zoomed one (`Tasks::visible_panes`). Checked on window activation, task or tab switches, focus and every hc event. The user's wish, 2026-11-25: after reading the answer there may be nothing more to do.
- Kept locally because nerdr can't reset hc's status. After a restart every done pane off screen is green again.
- No status text in the sidebar: the dot is enough (the user, 2026-11-25).

## Dropped

- No "Agent mangler" and no restart button: the user dropped them on 2026-11-25, "trenger det ikke". Spec #18 had them (stories 37-38): a task whose agent had exited stayed, with a button that ran `claude --continue`. They were built, then removed. When Claude exits, its pane closes like any other.
- superseded 2026-11-25: hooks via `--settings`. nerdr started the agent as `claude --settings <file>` with `UserPromptSubmit`, `Stop` and `Notification` hooks running `hc label "$HC_TERMINAL_ID" nerdr.agent=<status>`, as spec #18 decided. Replaced by hc's own status above. What was learned:
  - `/hooks` lists `--settings` hooks as `[Startup]` next to the user's `[User]` and `[Plugin]` hooks, on the same events: they add up and replace none. Observed with Claude Code 2.1.332. The docs say hooks merge across settings levels but don't name `--settings`.
  - A hook's stdout is read as JSON. `hc label` prints the terminal record when stdout isn't a tty, and Claude Code failed it: "Hook JSON output validation failed — reason: expected string, received null". Needs `>/dev/null`.
  - There's no hook between answering a permission prompt and the end of the turn, so asking stayed until Stop (`PostToolUse` would have been needed).
  - `Notification` without a matcher also fires on `idle_prompt`, a while after Stop.
  - A Claude started by hand has none of nerdr's flags or labels, so anything keyed on them misses it.
- `claude` is the user's wrapper `~/bin/claude`, which `exec`s the real binary, so the terminal finishes when Claude exits. It asks for an AWS team first, in the pane.
