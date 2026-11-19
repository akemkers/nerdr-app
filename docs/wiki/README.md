# Wiki

Facts we had to find out about the systems around nerdr: hc, GPUI, macOS, kitty, the shell. Each one cost a session to learn; the wiki keeps it from costing another. How nerdr itself works and what's left to do stays in `docs/spike.md`.

## Layout

- One page per topic, flat in this folder, kebab-case (`gpui.md`).
- `index.md`: one line per page, `- [[page]]: what it covers`.
- `log.md`: append-only, newest last. Entry header: `## [YYYY-MM-DD] <op> | <title>`.

Page format:

```markdown
---
sources: [commit 6929829, gpui-0.2.2/src/window.rs, https://...]
updated: YYYY-MM-DD
---
# Topic

- One fact per bullet, with its source: a commit, a `file:line`, a URL, or "observed: <how>".
```

## Rules

1. Extend first: search `index.md` and `rg -i '<term>' docs/wiki/` and add to the page that fits. A new page only for a new topic.
2. Every fact carries its source. Version-bound facts name the version (hc 0.1.21, gpui 0.2.2).
3. A fact that stops being true is marked `superseded YYYY-MM-DD: <what replaced it>` and kept.
4. Edit the section that changes; re-read the page right before.

## Operations

- **add**: after a task that taught something durable (a gotcha, an API quirk, why something is the way it is), add it to its page, update `index.md` if a page was added, append to `log.md`. Commit with the code it came from, or alone as `wiki: <title>`.
- **query**: read `index.md`, then the pages it points to. Check a fact against its source before building on it when the version may have moved.
- **lint** (when asked): pages missing from the index and vice versa, facts without a source, duplicates across pages, facts the code or a newer version contradicts.
