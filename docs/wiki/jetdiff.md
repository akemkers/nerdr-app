---
sources: [issue #21, issue #20, jetdiff v0.3.0 crates/jetdiff-ui/README.md, jetdiff v0.3.0 crates/jetdiff-ui/src/key_bindings.rs, jetdiff v0.3.0 crates/jetdiff-app/src/repo_source.rs, crates/nerdr/src/workspace.rs]
updated: 2026-11-25
---
# jetdiff in nerdr

The diff overlay (GLOSSARY.md) is jetdiff's `RepoView`, embedded. How it's wired, and what jetdiff's API asks of an embedder.

- The dependency is a git dep on a jetdiff tag (`v0.3.0`), `jetdiff-ui` and `jetdiff-app`. For work on both at once, an uncommitted `[patch."https://github.com/akemkers/jetdiff"]` can point at `~/git/jetdiff`. GPUI must be the same version in both, so `cargo tree -i gpui` shows one. Source: issue #18, observed with `cargo tree`.
- Setup at start, as jetdiff's own binary does: `Application::with_assets(jetdiff_ui::Assets)` (its file icons), `load_font`, `cx.bind_keys(key_bindings())`, and the global `Settings::new(&config, version)` from `jetdiff_app::config::load()` (the user's `~/.config/jetdiff/config.toml`: font size, folding, diff mode). If the config can't be read, the error goes to stderr and prefix+d says so. `main.rs` (`diff_setup`).
- jetdiff's keys are bound under the key context `jetdiff`, which `RepoView` sets, so `q`, `b`, `v`, `tab` don't reach past it into the terminals. Source: jetdiff v0.3.0 `key_bindings.rs`.
- jetdiff doesn't handle its `Quit` action (`q`, cmd-q, cmd-w). nerdr handles it with `on_action` on the div around the view, which is on the dispatch path from the focused element, and leaves the overlay.
- `RepoSource` isn't `Send` (it holds an `Rc<[ChangedFile]>`), and there is no public way to build it from a read done elsewhere, so the first open runs `RepoSource::open` on the main thread. Measured about 95 ms on nerdr-app and jetdiff (release build), 20 ms for a folder outside git. jetdiff runs its later reads in the background itself. Moving the first one off the main thread needs a change in jetdiff. Observed 2026-11-25.
- Outside a git repo `RepoSource::open` gives git's own error (`fatal: not a git repository …`); nerdr shows it in a popup that the next key closes.
- One `RepoView` per task, keyed by task id, made on the first prefix+d and kept while the app runs, so the file, scroll and folds are as you left them. It goes when its task does. Whether it's shown is the task's `diff` flag in `tasks.rs` (not saved). `workspace.rs`.
