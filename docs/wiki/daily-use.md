---
sources: [observed in sessions 2026-11-18/19, ~/.config/kitty/kitty.conf, CLAUDE.md, screenshots from the user]
updated: 2026-11-19
---
# How the user uses nerdr

What nerdr has to handle first, because the user meets it every day. When choosing what to fix or build next, this outranks completeness.

- **Coding agents in the terminal**, mainly Claude Code. Needs shift+enter for new lines (kitty protocol, [[input]]), and long-running agents in hc terminals that outlive nerdr.
- **A tmux user.** The controls should be tmux's, so they are recognised at once: prefix `C-f` (from `~/.config/tmux/tmux.conf`), `base-index 1`, own bindings like `v`/`h` for splits and `X` to kill a window. The user's own tmux-based nerdr (`~/git/nerdr`) is the model ([[input]]).
- **Norwegian keyboard layout.** On the Mac, `[]{}|@\` come from option, and `´¨` are dead keys. Anything that takes option or alt for shortcuts collides ([[input]], [[macos]]).
- **kitty is the daily terminal today** and the reference for how nerdr should look and behave ([[kitty]]). When something feels off in nerdr, comparing against kitty is the first check.
- **zsh with oh-my-zsh and Pure** (dot-files in `~/.config/dot-files`), fish set in kitty's config. Prompt and title behaviour come from there ([[shell-titles]]).
- **git TUIs and `git log --graph`**, which lean on box drawing and rounded corners ([[rendering]]).
- **jetdiff** is the user's diff tool and will become a pane in nerdr (`CLAUDE.md`).
- **Two machines:** the work Mac, where nerdr runs today, and Linux with Hyprland on Wayland, which is the primary target. Features have to work on both, and behave the same unless the platform forces otherwise (as with option on macOS).
