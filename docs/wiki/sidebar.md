---
sources: [issue #25, crates/nerdr/src/workspace.rs (render_card, render_tabbar), crates/nerdr/src/status.rs, crates/nerdr/src/worktree.rs (checkout), observed 2026-11-25 with hc 0.1.21]
updated: 2026-11-25
---
# The sidebar and the tab bar

What the sidebar shows for each task, and where the tabs went. Spec: issue #25 (parent #18).

## Cards

- One card per task: the repo, the branch, and a status dot with its age (`4m`, `2t`, `3d`, `<1m`). The active task's card is lit. Clicking a card shows the task as it was left, overlay and all (`Tasks::select` with no tab).
- The repo is the main worktree from `git worktree list --porcelain` (its first entry), so a nerdr worktree shows the repo it came from. Outside git: the origin's folder name, `~` for home. No branch line then.
- The branch is `git symbolic-ref --short HEAD`, or the short commit when detached. Read again for every origin every 2 s in the background (`CHECKOUT_EVERY`), so a branch the agent renames shows its new name. Chosen over a file watcher on HEAD as the smaller solution (2026-11-25). The same round re-renders, which moves the ages.
- A `wt` tag before the branch when the origin is a linked worktree: `git rev-parse --git-dir` and `--git-common-dir` differ. Kept plain on purpose; the user wants to style the sidebar later (2026-11-25).
- The dot is the most urgent status among all the task's panes; among equals the one that has been so the longest (`status::most_urgent`). No status words, the dot is enough ([[agent]]). No "Agent mangler", dropped in #24.
- The age is hc's `status_ms`: when the word last changed. It comes in `terminal.list`, the snapshot and `created`, but not in `status` events, so nerdr sets it to now when a status event changes the word. Observed with `hc events --json`, hc 0.1.21.

## Tab bar

- The active task's tabs, along the top of its area: dot, number (`base-index`), label, 🔍 when zoomed. The active tab is lit. Clicking one selects it, and leaves the diff overlay.
- It takes `TABBAR_H` (28 px) off the panes: `pane_area`, `grid_size` and dragging a line all count from under it.
- The diff overlay covers it too, as spec #18 says ("everything right of the sidebar", story 19). Chosen 2026-11-25.
