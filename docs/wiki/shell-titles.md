---
sources: [~/.zshrc (dot-files), ~/.local/share/zinit/plugins/sindresorhus---pure/pure.zsh, observed 2026-11-18]
updated: 2026-11-19
---
# Shell titles

- With oh-my-zsh and Pure both loaded, each command changed the title three times: Pure's `git: <cmd>` on preexec, then oh-my-zsh's `user@host:~/git` and Pure's `~/git` on precmd. A frame drawn between the last two flashed the middle one.
- oh-my-zsh stops with `DISABLE_AUTO_TITLE="true"`. Pure has no option; overriding `prompt_pure_set_title() { :; }` after loading it stops it. Both are in the user's `~/.zshrc`.
- nerdr now shows a fixed window title (`nerdr-app v<version>`) and uses program titles only for tab names, so shells that do set titles no longer flash it.
- To see what a shell really sends: spawn `zsh -i` in a pty (python `pty.fork`), send a command, and list the `ESC ] 0/1/2 ;` sequences in the output.
