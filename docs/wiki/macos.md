---
sources: [commits 6929829 28d7583 a040014, observed 2026-11-18/19]
updated: 2026-11-19
---
# macOS

- No `XDG_RUNTIME_DIR`. Anything that builds paths from it needs a fallback; see [[hc]] for hc's.
- Norwegian layout: `[` `]` `|` `@` are option+digit, `{` `}` option+shift+digit. Option must type characters, and shortcuts on alt+digit steal them. nerdr: option goes to the input method, tab jumping is cmd+1…9. Commits 28d7583, a040014.
- With option typing characters it is no meta key: alt+b / alt+f in the shell type special characters. Open question whether the user needs meta.
- An agent here can't see or drive the nerdr window: its terminal has no Screen Recording permission (`screencapture -l<id>` says "could not create image from window") and no Accessibility permission for keystrokes. Window ids come from `CGWindowListCopyWindowInfo` (a short swift script). Verify with logs (`NERDR_DEBUG`) and isolated hc sessions instead, or ask the user to look.
- `~/.zshrc` is a symlink into `~/.config/dot-files`. GNU `sed -i` replaces a symlink with a file unless given `--follow-symlinks`.
