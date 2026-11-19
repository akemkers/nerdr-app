# Spike: one hc terminal in a GPUI window

Status: 2026-11-06, hc 0.1.18, gpui 0.2.2, alacritty_terminal 0.26.0. Crate: `crates/nerdr` (~1500 lines).

```sh
cargo run --release              # reuse the live terminal labelled nerdr.spike=1, or start a shell with that label
cargo run --release -- t7        # attach to t7 (an address or a name)
NERDR_DEBUG=1                    # throughput per burst, unexpected frames, unanswered queries on stderr
NERDR_TRACE=/path/file           # write every DATA payload to a file
NERDR_FONT_SIZE=15               # default 16
HC_SOCKET / HC_SESSION           # another socket or session
```

Closing the window detaches; the shell keeps running in hc, and the next `cargo run` reattaches to it.

## What works

Verified with screenshots on a headless Hyprland output, input sent with `hc send`/`hc keys`, and compared with `hc read`:

| | |
|---|---|
| Connect, hello, framed mode, `terminal.list`/`run`/`attach` | yes |
| First paint, live output, CLOSE when the program exits (shown in the window title) | yes |
| 16 colours, 256, truecolor, OSC 4/10/11 overrides, bold, italic, bold-italic, dim, inverse, hidden, underline, strikethrough | yes |
| Wide characters (CJK), emoji, combining marks (approximate) | yes |
| Box drawing U+2500–257F, light and heavy (drawn as rectangles, ╭╮╯╰ as curves, so lines meet at any line height), block elements U+2580–259F (drawn as rectangles) | yes |
| Cursor: block, beam, underline, hidden; outline when unfocused | yes |
| btop, nvim (syntax, visual selection, statusline), `ls --color`, `clear` | yes |
| Resizing the window: RESIZE frame → the core resizes the pty → the program redraws | yes (tiled a second window next to it; 118×25 → 58×25, matching `hc get`) |
| Floods: `yes \| head -1000000`, `seq 1 1000000`, 400k coloured lines with CJK | yes, coherent mid-flood |
| Scrollback replayed on attach (`history: 5000`) | yes |
| OSC 0/2 titles name the tab in the sidebar; the window title is fixed (`nerdr-app v<version>`) | yes |

Tested by hand on real hardware (2026-11-09, Norwegian layout): typing latency and key repeat feel right; AltGr characters, ctrl+arrows, wheel scrollback, selection, ctrl+shift+c and ctrl+shift+v work.

Implemented; only the parts above were tested by hand:

- Keyboard: letters, shift, ctrl (a–z and `@[\]^_`), alt (ESC prefix), arrows/home/end with modifiers (`CSI 1;m X`, SS3 in app-cursor mode), pgup/pgdn/ins/del, F1–F12, enter, tab, shift-tab, esc, backspace (ctrl → BS). Unit tests cover the encoding. AltGr is xkb's Mod5, not Alt, in GPUI, so AltGr characters on a Norwegian layout (`{[]}\|@$`) arrive as `key_char` without an ESC prefix.
- Kitty keyboard protocol when a program asks for it (alacritty tracks the flags; the core answers `CSI ? u` and restores the flags in the paint): all five flags, so shift+enter is `CSI 13;2u`. Not reported: lone modifier keys and the base layout key (GPUI gives neither), lock modifiers. Releases go out only for keys whose press did.
- Scrollback with the wheel, shift+pgup/pgdn. In the alternate screen the wheel sends arrow keys (alternate scroll), and with mouse reporting on, it sends wheel buttons.
- Mouse reporting (X10 and SGR: press, release, drag, motion) when the program asks for it; shift bypasses it.
- Selection: click-drag, double-click (word), triple-click (line). ctrl+shift+c copies to the clipboard; ctrl+shift+v pastes (bracketed when the program asked for it).
- FOCUS frames on window focus in/out.

## What doesn't work (yet)

- Keystroke-to-glyph latency not measured (feels fine by hand).
- Middle-click may never reach nerdr: on Linux, GPUI did not call the middle-button handler when tested. If so, programs with mouse reporting (vim `mouse=a`, tmux) get no middle-clicks either. Not investigated.
- No IME (GPUI's `InputHandler` isn't wired). Dead keys/compose come through GPUI's own compose handling.
- No hyperlinks (OSC 8), images (kitty/sixel), undercurl styles (every underline style is drawn as one line), cursor blink, bell, URL detection.
- `display` is sent only on attach. GPUI reports scale factor 1 before the window is on an output, so the cell size given to the core can be in logical pixels (`CSI 16t` answered 20×8 instead of 41×17 at scale 2). There's no call to update `display` later; re-attach would be the fix.
- Resize reflows in two places, alacritty in the client and Ghostty in the core. They can disagree on soft-wrapped lines until the program redraws or we re-attach.

## Protocol details learned

The OpenRPC schema (`hc schema`) names the frames but not their wire format or kind numbers. I recorded those from `hc attach` through a logging Unix-socket proxy (`XDG_RUNTIME_DIR` pointed `hc` at it).

**Socket.** `$HC_SOCKET`, else `$XDG_RUNTIME_DIR/herdr-core/<session>.sock`, else `$XDG_STATE_HOME/herdr-core/<session>/<session>.sock` (`~/.local/state` by default, the usual case on macOS). Session `default` unless `HC_SESSION` is set. Paths over 107 bytes are refused (`hc` says so; it matters for tests in deep temp dirs). The directory also holds `default.holder.<id>.sock`, the holder process, not for clients.

**Handshake.** Line mode first, one JSON message per line:

```json
→ {"jsonrpc":"2.0","id":0,"method":"hello","params":{"generation":1,"framed":true}}
← {"id":0,"jsonrpc":"2.0","result":{"core":{"id":"…","name":"zephyrus"},"features":["framed","attach","events","status","shell","resize_notifications","automatic_input"],"framed":true,"generation":1,"methods":[…],"version":"0.1.18"}}\n
```

After that newline everything in both directions is framed. `hc attach` itself sends no `generation`.

**Frame.** 9-byte header, little endian:

```text
u32 channel | u8 kind | u32 payload length | payload
```

Channel 0 is JSON-RPC: kind 0, one message per frame, no trailing newline. Requests, replies and notifications all go there.

**Stream channel** (`channel` from `terminal.attach`, 1 for the first attach on a connection):

| kind | name | direction | payload |
|---|---|---|---|
| 0 | DATA | core → viewer | terminal output bytes: first the paint, then live output |
| 0 | DATA | viewer → core | input bytes, as typed (`echo hi\r`) |
| 1 | CREDIT | viewer → core | u32 LE: DATA bytes consumed |
| 2 | CLOSE | core → viewer | JSON `{"exit_code":0,"finished_ms":…,"reason":"exited","signal":null}` |
| 2 | CLOSE | viewer → core | empty: detach; the core answers with an empty CLOSE and stops sending |
| 3 | RESIZE | viewer → core | u16 LE cols, u16 LE rows |
| 4 | FOCUS | viewer → core | 1 byte: 1 focused, 0 not |
| 5 | FACTS | ? | never seen; numbered by elimination |
| 6 | AUTOMATIC_INPUT | viewer → core | input that isn't a person typing (attach result `automatic_input: true`) |

Recorded examples (hex):

```text
01000000 01 04000000 d0010000        CREDIT 464 bytes
01000000 03 04000000 6400 1e00       RESIZE 100×30
01000000 04 01000000 01              FOCUS in
01000000 00 08000000 6563686f206869 0d   DATA "echo hi\r"
```

**Credit.** `hc attach` credits each DATA frame by its length, right after writing it. Without credit the core stops after ~256 KiB (measured: stall at 261 407 bytes). The window is per attach. nerdr credits once per batch of frames it parsed.

**Paint.** The first DATA after attach is a full state replay, safe for any VT parser:

```text
CAN, ESC[!p (soft reset), reset every mode (?2026l ?1000l … ?1049l, ESC[=0;1u kitty flags),
ESC[H ESC[2J, tab stops, then `history` lines of scrollback and the screen as text with SGR,
cursor position, OSC 2 title, modes the program set (?2004h, ?1004h, …)
```

The scrollback lines are printed and scroll off the top, so they land in the client's own history. A resize by an active viewer doesn't trigger a new paint (`resize_notifications: false`); the program redraws on SIGWINCH.

**Queries are answered by the core.** The core runs libghostty-vt per terminal and answers DA, CPR, `OSC 10/11/12 ?` and `CSI 14t/16t` itself, using the `display` given to `terminal.run`/`terminal.attach`. They don't appear in the stream. A viewer must not answer (alacritty_terminal's `PtyWrite`/`ColorRequest` events are dropped), or the program would get two replies. Measured inside the spike's shell: `DA=\E[?62;22c`, `OSC 11 → rgb:1a1a/1b1b/2626` (our background).

**Other observations.**
- Live DATA frames are tiny under load: median 11 bytes (`yes`), 30 bytes (`seq`), max 16 KiB. That's ~100–300k frames/s, so per-frame cost matters. Reading everything already buffered, parsing it under one lock and sending one CREDIT is what made the client keep up.
- `terminal.attach` returns the whole `Terminal` (argv, cwd, labels, foreground process, title, status), enough for a tab label without another call.
- `terminal.run` with `no_parent: true` is needed when nerdr itself runs inside an hc terminal; otherwise the new shell becomes that terminal's child and dies with it.
- `hc send` pastes (bracketed) and presses enter; in nvim's normal mode the text is inserted. Use `hc keys` for keystrokes. `hc keys` doesn't take `:`.
- An attach whose client stdin hits EOF sends `^D`: my first probe killed its own shell that way.

**Several streams on one connection.** Each `terminal.attach` gets the next channel (1, 2, …), even for the same terminal twice. `events.subscribe` gets one too.

**Events.** In framed mode, `events.subscribe` returns `{channel, seq, terminals}` (the snapshot). Events then arrive as DATA on that channel, credited like terminal output: JSON lines, `{"jsonrpc":"2.0","method":"event","params":{"type":"finished","terminal":"t39","seq":360,"data":{…}}}`. Types seen: created, foreground, finished, size. `missed` and `heartbeat` are described in the schema but haven't been seen yet.

**Core restart** (`hc server restart`) ends the socket (EOF) with no CLOSE frames first. A new connection works after about 100 ms, and attaching again gives a fresh paint. The programs keep running.

## Connection layer

`hc::Client` (`crates/nerdr/src/hc.rs`): one socket shared by every terminal and the event feed.

- `call(method, params)` returns a future. Replies are matched by id. `call_blocking` is for startup.
- `attach(params, sink) -> Stream`. Output goes to the `Sink` on the reader thread, one call per batch, then one CREDIT per channel. `Stream` sends input, resize and focus. Dropping it detaches (CLOSE).
- `notices()` reports Connected, Disconnected, Snapshot and Event. A new listener first gets the current state: Connected, the last snapshot and the events since it. After 1000 events the client subscribes again to get a fresh snapshot.
- Reconnect: on EOF it fails pending calls, tells each sink `Lost`, redials with backoff from 50 ms to 2 s, then subscribes and attaches every open stream again (with its latest size). A terminal view starts from an empty term on a repaint, so the history isn't doubled. A stream closed by the core is attached again after its terminal's `restarted` event.
- Writes carry the connection generation, so a write meant for a dropped connection never reaches the new one.
- Test: `cargo test --release -- --ignored` starts its own core (`HC_SESSION=nerdr-test-<pid>`). It checks two terminals on one connection, input, events, a flood well past the credit window, detach, core restart with reattach and input after it, and a terminal ending. It passed 10 of 10 runs.
- Throughput is unchanged from the spike: `seq 1 1000000` gives 10–11 MiB/s for both builds in a 798×972 window, against 14.6 MiB/s for a raw client that only credits.

## Spaces and tabs

`crates/nerdr/src/workspace.rs`. A space is a group of tabs, and a tab is one hc terminal. The sidebar on the left lists the spaces with their tabs under them. Clicking an entry selects it.

- **Layout.** Saved in the hc document `nerdr` (`doc.set`, versioned) on every change. On start, tabs whose terminal is gone are dropped, then empty spaces. With nothing left, a shell is started. `nerdr t7` adds t7 as a tab.
- **Labels.** A space is named after the last part of its active tab's directory (`~` for home). A tab shows its title, else the program's name, else its address. Both come from the event feed. A dot shows the OSC 7501 status: working is yellow, blocked and error are red, done is green, idle is grey.
- **Lifecycle.** When a program exits, its tab closes and the terminal is removed (`terminal.rm`). ctrl+shift+w kills and removes it. Closing the last tab quits the app. Hidden tabs stay attached, so switching is instant.
- **Keys.** Taken in the capture phase, before the terminal sees them: ctrl+shift+t new tab (in the active tab's directory), ctrl+shift+n new space (in home), ctrl+shift+w close tab, ctrl+tab / ctrl+shift+tab next and previous tab, alt+1…9 select tab, ctrl+shift+down / ctrl+shift+up next and previous space.
- **Verified** on a headless Hyprland output against a private core, with keys sent by `hyprctl dispatch "hl.dsp.send_shortcut({ mods = 'CTRL SHIFT', key = 't', window = 'pid:<pid>' })"` (the window doesn't need focus). Checked: new tab and new space, switching, `exit` closing a tab, ctrl+shift+w, the layout and status dots after a restart, and the app quitting on its last tab. Clicking in the sidebar wasn't tested.
- **Not done.** Renaming spaces, reordering tabs, and limiting background tabs (each one parses its output even when hidden).
- **Note on OSC 7501:** the report format is `ESC ] 7501 ; state=working:app=x BEL`, fields separated by `:`. The status event's data is `{"status": {...}, "records": {...}, "reported": true}`.

## Terminal emulation: alacritty_terminal

Kept. It is the parser and grid Zed's terminal uses with GPUI; it gives scrollback, selection, reflow on resize, modes and a damage-free "render the whole viewport" API that was fast enough. It worked first time with the hc paint.

The alternative worth testing before the MVP is **libghostty-vt in the client** (C ABI, MIT): the core already runs it, so the client would match the core exactly (reflow, kitty keyboard flags, graphics later). Cost: a Zig build in the Rust workspace and an unproven API. The two-emulator drift above is the main reason to look.

## Rendering in GPUI

- A `canvas` element. Prepaint locks the term, walks the viewport once, collects background rectangles (merged per run), underline/strike lines, block elements, the cursor and text segments, then unlocks and shapes. Paint draws quads then lines.
- Text is shaped per segment (a run of narrow cells with the same font, or one wide cell), with `force_width = cell width`, so glyphs stay on the grid whatever the fallback font. GPUI's per-frame line-layout cache makes unchanged lines nearly free.
- Cell size: advance of `m`, height = (ascent + descent), rounded up, × 1.3 (like kitty with `modify_font cell_height 130%`), both snapped to device pixels. Hack Nerd Font Mono when installed (as in kitty), else the embedded JetBrains Mono (the same files as jetdiff). Ligatures off.
- **GPUI 0.2.2 bug**: `TextSystem::layout_line` merges neighbouring runs that have the same colour and decoration even when their fonts differ, so `\e[1m` text in the default colour came out regular. Worked around by starting a new segment at every font change. Worth reporting upstream; jetdiff may hit it too.

## Performance

Release build, 960×514 logical window (118×25 cells) at scale 2:

| test | raw Python client (credit only) | nerdr |
|---|---|---|
| `yes \| head -1000000` (2.9 MiB) | 507 ms, 5.6 MiB/s | 662 ms, 4.3 MiB/s |
| `seq 1 1000000` (7.5 MiB) | 504 ms, 14.9 MiB/s | 496 ms, 15.2 MiB/s |
| 400k coloured lines with CJK (15.5 MiB) | – | 3.07 s, 5.0 MiB/s |
| `cat` 3× Cargo.lock (631 KiB) | – | 21 ms |

- The core is the limit, not the client: nerdr matches a client that does nothing but credit. Before batching, the debug build managed 1.5 MiB/s.
- CPU during a flood: nerdr ~50% of one core. RSS ~85 MB, binary 26 MB (release, unstripped).
- The reader sets a dirty flag and wakes the UI at most once per render; the UI draws the latest grid and skips everything in between. Screenshots mid-flood were coherent. Frame times not measured.
- Not measured: keystroke-to-glyph latency. Add a timestamped echo test (DATA out → DATA in → presented frame) before tuning anything.

## Effort to MVP

For one person who knows the code; GPUI is the uncertainty (thin docs, 0.2 API).

| piece | estimate | notes |
|---|---|---|
| Harden the terminal view | 4–6 days | IME, kitty keyboard, latency test, cursor blink, OSC 8, re-attach on core restart, `display` on scale change, keyboard/mouse tests on real hardware |
| ~~Connection layer~~ | done (2026-11-09) | one connection with many stream channels, a request/reply map instead of blocking calls, `events.subscribe`, reconnect |
| ~~Tabs and spaces~~ | done (2026-11-09) | model in the app, persisted with labels (`nerdr.space=…`) or `doc.set`, tab bar, keybindings |
| Splits | 3–5 days | split tree, resize handles, focus movement; one attach per visible pane, detach hidden ones (or `observe`) |
| Agent list from `events.subscribe` | 2–3 days | sidebar: status (`idle/working/done/blocked/error` from OSC 7501), title, cwd, jump to it, notifications for blocked |
| jetdiff-ui pane | 2–4 days | gpui versions already match; pick the root (`RepoView`/`FileDiffView`), share fonts/theme/assets, follow the terminal's cwd |
| Theme from Omarchy, config, packaging | 2–3 days | |
| **Total** | **~4–5 weeks** | the terminal itself is no longer the main risk |

## hc API and licence notes

- `hc --licenses`: hc is Apache-2.0 (also in the OpenRPC `info.license`). It bundles Ghostty's libghostty-vt (MIT) with simdutf, Highway and Wuffs (MIT/Apache/BSD-3), Zig compiler_rt (MIT), statically linked musl (MIT and BSD), the Rust std and about 50 crates (MIT/Apache, Unlicense OR MIT, Unicode-3.0). Nothing copyleft. A client that talks to the socket isn't affected either way. Ghostty's licence matters only if we link libghostty-vt ourselves (MIT: keep the notice).
- The licence text says "See distribution/notices/README.md in the source", so source exists, but this machine only has the package binary.
- Surprises, all mentioned above: the frame format and kind numbers aren't in the schema; the core answers terminal queries itself, so viewers mustn't; CREDIT is per byte with a ~256 KiB window; DATA frames are tiny under load; `generation` in `hello` is optional.
