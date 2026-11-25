# Index

nerdr:

- [[daily-use]]: how the user uses nerdr, and so what it must handle first
- [[input]]: how keys, IME, option/alt, mouse, clipboard and links reach the program, and why
- [[rendering]]: font and cell size, colors, box drawing, cursor and overlays, and why
- [[window]]: window size and the fixed title, and why
- [[testing]]: how an agent tests nerdr without touching the user's terminals, and what to hand to the user
- [[agent]]: the agent's status from hc (OSC 7501), done until seen, and the hooks tried before

Systems around nerdr:

- [[hc]]: herdr-core: install, socket path, labels and a terminal's environment, attach, paint, display
- [[gpui]]: GPUI 0.2.2 quirks: keys and IME flow, window title, fonts, platform-only APIs, drawing
- [[macos]]: macOS specifics: Norwegian layout and option, missing XDG dirs, what an agent can't do here
- [[kitty]]: kitty as the reference look and the kitty keyboard protocol
- [[shell-titles]]: why the window title flashed on every command (oh-my-zsh and Pure)
