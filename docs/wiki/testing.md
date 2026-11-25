---
sources: [observed 2026-11-18/19 while testing commits 2ce9a62..2712d18, crates/nerdr/src/view.rs (NERDR_DEBUG), observed 2026-11-25 dumping Claude Code 2.1.331 in a pty]
updated: 2026-11-25
---
# Testing nerdr as an agent

How an agent checks a change without disturbing the user's terminals, and what it has to hand to the user.

## Checks without the app

- `cargo build --release`, `cargo test`, `cargo clippy --release`. Clippy's one standing warning is the type of `block_element` in `view.rs`.
- Pure logic (key encoding, box drawing geometry) gets unit tests in its module.

## Running nerdr against an isolated hc session

The user's own nerdr is usually attached to the `default` session. A second nerdr there is an active viewer too: it resizes the user's terminals to its window (one test run took `t7` to 319 columns while the user had it open), and `nerdr <address>` adds a tab to the saved layout. So tests run in their own session:

```sh
export HC_SESSION=nerdrtest
hc server start
T=$(hc run --bg --json "sh /tmp/script.sh" | grep -o '"address":"[^"]*"' | cut -d'"' -f4)
./target/release/nerdr $T > /tmp/nerdr-test.log 2>&1 & P=$!; sleep 4; kill $P
hc server stop; rm -rf ~/.local/state/herdr-core/nerdrtest
```

- `HC_SESSION=<name>` is a separate core with its own terminals and state in `~/.local/state/herdr-core/<name>/`.
- `hc server status` starts the core when none is running, so checking a stopped test session brings it back. Stop it again. Observed with hc 0.1.21.
- `hc run --bg --json "<cmd>"` prints `{"address":"t2",...}`; without `--bg` it waits like a shell.
- `hc send <t> <text>` pastes and presses enter. Into a terminal the user is typing in, the texts merge (`clear` + `ls -la` ran as `clearls -la`), so send only to test sessions.

## A real Claude in a test session

- Start the test core without the agent's own Claude session variables, or the Claude in it won't save its transcript ([[hc]]): `env -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT -u CLAUDE_CODE_MESSAGING_SOCKET -u CLAUDE_CODE_MESSAGING_TOKEN -u CLAUDE_CODE_SESSION_ID -u CLAUDE_CODE_CHILD_SESSION -u CLAUDE_CODE_SESSION_ATTENDED -u CLAUDE_PID -u CLAUDE_CODE_CHILD_NOTES_DIR -u CLAUDE_CODE_EXECPATH HC_SESSION=nerdrtest hc server start`. Observed 2026-11-25.
- The user's `claude` wrapper asks for an AWS team: `hc send <t> aksjehandel`. A new folder asks for trust: `hc keys <t> down enter`. `hc keys <t> shift+tab` cycles the permission mode, to get a permission prompt out of auto mode.
- `hc events --json` in the background shows status and labels events as they come.
- The saved layout without the app: `echo '{"jsonrpc":"2.0","id":1,"method":"doc.get","params":{"name":"nerdr"}}' | nc -U -w 2 ~/.local/state/herdr-core/nerdrtest/nerdrtest.sock`.

## Seeing what happens

- `NERDR_DEBUG=1` logs hc events, stream closes and display queries nerdr leaves unanswered.
- For anything else, a temporary `eprintln!` at the spot, run, read the log, then restore the file from a copy made before (`cp view.rs /tmp/view.rs.keep`). Found this way: the title flashing was two titles 30 ms apart ([[window]]); the attach paint carries no OSC 8 ([[hc]]).
- What a shell sends (titles, modes): spawn `zsh -i` in a pty with python `pty.fork`, send a command, print the escape sequences. See [[shell-titles]].
- What Claude Code draws: the same pty trick with `/opt/homebrew/bin/claude` (the `claude` on PATH is an alias plus a wrapper that first asks for an AWS team), started in a folder it already trusts, else it stops at the trust dialog. Read for 8 s, then kill it. Observed 2026-11-25 with Claude Code 2.1.331.

## What only the user can check

The agent can't type into the nerdr window or capture it on this Mac ([[macos]]). For changes to typing, looks or mouse, end with a short list of concrete things for the user to try (keys to press, a command to run, what should happen), and say plainly that it's untested in the app.

## Clean up

Stop test sessions and delete their state folder, kill test nerdr processes (`pgrep -x nerdr` also lists the user's own), remove `/tmp` logs and backups.
