# nerdr-app

A desktop app for working with coding agents: each piece of work gets its own place with a Claude, terminals and the diff of what changed.

## Language

**Task**:
One piece of work: an agent started from an origin, with the tabs and panes opened around it. Shown as one entry in the sidebar.
_Avoid_: space (the name in the code today), session, chat

**Agent**:
The Claude a task is started with, running in one of its panes. A task can be without its agent for a while and still be the same task.
_Avoid_: chat, Claude session

**Origin**:
The folder a task starts from and belongs to, fixed for the task's life. A worktree made for the task, or the repo itself.
_Avoid_: cwd, root

**Diff overlay**:
jetdiff showing the origin's changes, covering the task's tabs until you leave it. Each task has its own, kept while the app runs.
_Avoid_: diff pane, diff view

**Tab**:
One of a task's views, a tree of panes. tmux's window.

**Pane**:
One terminal in a tab, run by hc.
