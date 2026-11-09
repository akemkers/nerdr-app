# LLM Wiki: research notes

Researched 2026-11-06. Star counts are from the GitHub API on that date.

## Summary

1. "LLM Wiki" mainly means Andrej Karpathy's pattern, published as a gist on 2026-04-04. An LLM agent builds and maintains an interlinked Markdown wiki from immutable raw sources. [G]
2. It has three layers: `raw/` (sources the agent never edits), `wiki/` (pages only the LLM writes) and a schema file (`CLAUDE.md`/`AGENTS.md`). Two files help navigation: `index.md` (catalog) and `log.md` (append-only timeline). [G]
3. It has three operations. Ingest a source (one source touches 10–15 pages), query (good answers get filed back as pages) and lint (contradictions, stale claims, orphans, missing links). [G]
4. It works without RAG up to ~100 sources and hundreds of pages. Past that, add local search (`qmd`). The known failures are near-duplicate pages, silent overwrites, unverified claims, token cost of routing, and staleness. [G][GC][VB]
5. With several agents, use one canonical `AGENTS.md`, git as the safety net, per-agent inbox drafts, and a single writer for index/log. Have agents append to inboxes rather than edit pages concurrently. Neighbours: Claude Code auto memory (per-repo notes), RAG/NotebookLM (big corpora), and session-transcript tools.

Source keys are listed in [Sources](#sources).

---

## 1. What "llm-wiki" refers to

| Thing | What it is | Link | Date / size |
|---|---|---|---|
| **Karpathy "LLM Wiki" gist (main meaning)** | An "idea file" meant to be pasted into an agent (Codex, Claude Code, OpenCode/Pi). It describes the pattern, not an implementation. | https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f | Created 2026-04-04, 1 revision, 5k+ stars and forks [GC] |
| Karpathy tweet "LLM Knowledge Bases" | The original post: Obsidian as the front end, `raw/` compiled by the LLM into a wiki, LLM "health checks". Reported scale is ~100 articles and ~400k words. | https://x.com/karpathy/status/2039805659525644595 | Unverified (could not read X): 2026-04-02. VentureBeat covered it on 2026-04-03. [VB] |
| "LLM Wiki v2" (rohitg00) | Extension gist. Adds confidence per claim, decay over time, memory tiers (working/episodic/semantic/procedural) and an audit trail. | https://gist.github.com/rohitg00/2067ab416f7bbe447c1977edaaa681e2 | 2026-04. Unverified: content seen only through summaries. |
| nashsu/llm_wiki | Desktop app (cross-platform). Its ingest queue is serial and persisted to disk. | https://github.com/nashsu/llm_wiki | 21.4k stars |
| AgriciDaniel/claude-obsidian | Claude Code plugin for Obsidian vaults, with transactional writes and conflict detection. | https://github.com/AgriciDaniel/claude-obsidian | 16.2k stars |
| SamurAIGPT/llm-wiki-agent | Template for Claude/Codex/Gemini with `/wiki-ingest`, `/wiki-query`, `/wiki-lint`, `/wiki-graph`. | https://github.com/SamurAIGPT/llm-wiki-agent | 3.7k stars |
| Astro-Han/karpathy-llm-wiki | A single Agent Skills skill (`npx add-skill`) with a well-argued list of "deliberately not built" features. | https://github.com/Astro-Han/karpathy-llm-wiki | 2.7k stars |
| atomicstrata/llm-wiki-compiler | CLI "knowledge compiler". Includes `ingest-session` for Claude/Codex/Cursor exports. | https://github.com/atomicstrata/llm-wiki-compiler | 2.2k stars |
| nvk/llm-wiki | Plugin for Claude Code, Codex and OpenCode. Parallel research agents, plus capture of session hooks into `.sessions/` with a promote step. | https://github.com/nvk/llm-wiki | 1.5k stars |
| zosmaai/pi-llm-wiki | Extension for **pi**. Personal vault (`~/.llm-wiki/`) plus project vault (`.llm-wiki/`), guardrails, qmd index and an MCP server. | https://github.com/zosmaai/pi-llm-wiki | 625 stars |
| pratiyush/llm-wiki ("llmwiki") | Turns Claude Code, Codex, Cursor and Gemini session transcripts into a wiki and static site. | https://github.com/pratiyush/llm-wiki | Unverified: GitHub API timed out. README read. |
| tobi/qmd | Local Markdown search engine (BM25 + vector + rerank), CLI and MCP. The gist recommends it. | https://github.com/tobi/qmd | 30.9k stars, MIT |

The many smaller repos named `llm-wiki` (weij, KaelLim, odere-pro, and others) are templates or skills for the same pattern.

## 2. How it works (per the gist [G])

**Layers**

| Layer | Owner | Rule |
|---|---|---|
| Raw sources (`raw/`) | Human curates | Immutable. The LLM reads but never modifies. This is the source of truth. |
| Wiki (`wiki/`) | LLM | The LLM creates and updates summaries, entity, concept and comparison pages, the overview and the synthesis, and keeps cross-references consistent. The human reads. |
| Schema (`CLAUDE.md` / `AGENTS.md`) | Human and LLM co-evolve it | Structure, conventions, and the workflows for ingest, query and lint. Karpathy calls it "the key configuration file". |

**Special files**

| File | Purpose |
|---|---|
| `index.md` | Content catalog. Each page gets a link, a one-line summary and optional metadata, grouped by category. Updated on every ingest. Read first when answering a query. |
| `log.md` | Chronological and append-only, covering ingests, queries and lint passes. Use the heading prefix `## [YYYY-MM-DD] ingest \| Title` so that `grep "^## \[" log.md \| tail -5` works. |

**Operations**

| Op | Steps |
|---|---|
| Ingest | Read the source, discuss takeaways with the human, write a summary page, update `index.md`, update related entity/concept pages, append to `log.md`. Karpathy ingests one source at a time and stays involved. Batch ingest is possible. |
| Query | Read the index, open the relevant pages and answer with citations. The output can be a page, table, Marp deck, chart or canvas. **File good answers back into the wiki.** |
| Lint | Look for contradictions, stale claims, orphan pages, concepts without a page, missing cross-links and data gaps (fill those by web search). Suggest new questions and sources. |

**What the schema contains:** the folder layout, page types and formats, frontmatter, link style, the step-by-step workflow for each operation, and the user's preferences, such as how much to involve the human. [G]

**Tooling in the gist:**
- Obsidian as the reader ("Obsidian is the IDE; the LLM is the programmer; the wiki is the codebase").
- Graph view and Dataview (queries over frontmatter).
- Marp for slide decks.
- Web Clipper for capturing sources.
- Download images to `raw/assets/` (Settings → Files and links → attachment folder; bind a hotkey to "Download attachments for current file").
- The wiki is a git repo. [G]

## 3. Setting it up effectively

**Decisions**

| Decision | Recommendation | Why |
|---|---|---|
| Location | A standalone git repo (e.g. `~/git/wiki`), opened as its own Obsidian vault. Keep it separate from `~/Documents/Obsidian/Main`. | Agents get a sandbox with full git history, and the personal vault stays untouched. Obsidian can open several vaults. |
| Schema file | Make `AGENTS.md` canonical, with a `CLAUDE.md` that contains only `@AGENTS.md`. | Codex and pi read `AGENTS.md`. Claude Code loads `AGENTS.md` only when there is no `CLAUDE.md`, and expands `@path` imports [CCM]. Unverified: pi's exact loading rules. |
| Schema size | Under ~200 lines. Put detail in `docs/` pages that the agent reads on demand. | Claude Code's guidance is that longer instruction files reduce adherence [CCM]. |
| Search | `index.md` plus `rg` until recall drops, then `qmd` (CLI or MCP). | At 50k–100k tokens of curated wiki, grep and read are more reliable than vectors [AH]. The index alone covers ~100 sources [G]. |
| Ingest mode | Interactive, one source at a time, while the schema is young. Batch later. | [G]. |
| Verification | Scripted checks (links, citations, a quoted value must exist in the source) rather than model self-checks. | Commenters report that models rarely verify on their own, and code-based link and citation lint worked [GC]. |
| Obsidian licence | Free for work since 2025-02-20. | [OB]. VentureBeat's claim that a commercial licence is needed is outdated [VB]. |

**What worked (reported)**
- "Disagreements & open questions" sections were the most useful part of an unattended podcast wiki (69 concept pages from 60 episodes). [GC, HagaiHen, 2026-10-08]
- A daily production wiki with 94 articles and 99 sources. The author deliberately skipped confidence scores, review dates, vector search, hooks and typed ontologies. [AH]
- Lint checks computed in code, for broken citations and links. [GC, frankchu91, 2026-10-16]
- One serial ingest queue, persisted to disk with retry. [NS]

**Known pitfalls**

| Pitfall | Mitigation | Source |
|---|---|---|
| Near-duplicate pages | "Extend, don't create": search the index and `rg` before creating a page, and run periodic merge passes in lint. | [GC] HagaiHen 2026-10-08 |
| Silent overwrites | Read before write, compare a hash before applying, keep one plan per change, commit per operation. | [GC], [CO] |
| Hallucinated or unverified claims | Every claim links to a `raw/` source. Lint greps that the quoted value exists in the source. | [GC] ShootJackal 2026-10-01, NDOTO-G 2026-10-27; [AH] |
| Compiled pages lack detail, so agents fall back to raw sources | Keep `raw/` and let queries drill into it. Pages summarise; they don't replace sources. | [GC] hyunsoo36 2026-10-30 |
| Token cost (one report: ~72% of tokens spent on routing during ingest) | Cheap routing (index plus grep, or a small model). Ingest only what matters. | [GC] sassarini-marco 2026-10-15 (one corpus, unverified) |
| Staleness, superseded facts | Keep a `## Contradictions / superseded` section on each page. Lint flags it. Don't delete; mark it superseded with the date and source. | [G], [GC] liushengyu 2026-11-02, [V2] |
| Hallucinations compounding in shared multi-agent memory | Use a review gate or a separate validation pass before content is promoted into `wiki/`. | [VB] |
| Scale | The index works to ~100 sources and hundreds of pages. Beyond that, use real search. Expect it to get "brutal" at enterprise scale with contradictory tribal knowledge. | [G], [VB] |
| Markdown fits relational code knowledge (e.g. "what calls X") poorly | Leave call graphs to code tools. The wiki holds decisions and the reasons behind them. | [GC] bg-numarqe 2026-10-05 (promotional) |

## 4. Coding agents and several agents writing to one wiki

Primary-source patterns:
- **claude-obsidian:**
  - Read every target note and record its SHA-256.
  - Helpers work in parallel but hand back drafts only. One coordinator merges them into one plan and applies it as a single transaction, with a journal for rollback.
  - If a hash changed in the meantime (for example, the human edited the note in Obsidian), the write stops with a conflict.
  - The lock coordinates only cooperating local writers. [CO]
- **pi-llm-wiki:**
  - Blocks agent edits to `raw/**` and to generated metadata (`index.md`, `log.md`, registry, backlinks).
  - Rebuilds that metadata from the pages at the end of each turn.
  - Has two layers: a personal vault plus a per-project vault. [PI]
- **nvk/llm-wiki:**
  - Hooks capture small redacted session digests into `.sessions/`.
  - Corrections and decisions become *reviewable candidates*, which an explicit `promote` step moves into the wiki. [NV]
- **nashsu/llm_wiki:** the ingest queue is serial, to avoid concurrent LLM writes. [NS]
- **Transcript tools** build from what agents already save: pratiyush/llm-wiki and llm-wiki-compiler's `ingest-session` convert Claude Code and Codex transcripts. [PR], [AC]

Recommended conventions. These synthesise the patterns above; none of it is a published standard.
1. **Agents don't edit `wiki/` pages directly during task work.** At the end of a session, an agent writes one file: `inbox/YYYY-MM-DD-HHMM-<agent>-<slug>.md`, containing facts, decisions and sources (file paths, commits, URLs). Unique file names mean no write conflicts.
2. **One integrator at a time.** A dedicated `ingest inbox` run does three things:
   - Merges inbox files into pages ("extend, don't create").
   - Moves the processed inbox files to `raw/sessions/`, where they are immutable.
   - Appends to `log.md`.

   Serialise it with a lock file (`.wiki.lock` holding the agent name and timestamp; give up and retry if it exists).
3. **Index and log are derived or append-only.** `log.md` only ever gets entries appended. Better still, regenerate `index.md` with a script from frontmatter, so two agents never hand-edit it.
4. **Git as the safety net.** Before writing, check `git status`. Make one commit per operation (`ingest: <title>`, `lint: ...`, `inbox: <agent> <slug>`), with no rebases or force pushes. If the wiki is synced across machines, `git pull --rebase` before every operation.
5. **Read before write:** re-read a page immediately before editing it, and edit sections with exact-match replacements, never whole-file rewrites.
6. **Provenance on every page:** `sources:` frontmatter, plus `updated_by: <agent>` and a date, so lint and humans can trace who wrote what.
7. **Content that belongs in the wiki:** durable cross-project knowledge, such as tool and API facts (e.g. `hc` protocol quirks), decisions and their reasons, gotchas and how-tos. Project-only instructions stay in that repo's `AGENTS.md`/`CLAUDE.md`. Ephemeral state stays out.

Hooking into the user's agents:
- **Claude Code:**
  - A `Stop`/`SessionEnd` hook or a skill (`/wiki-retro`) that writes the inbox file.
  - The wiki path goes in `~/.claude/CLAUDE.md`, e.g. "for cross-project knowledge, read `~/git/wiki/index.md`".
  - Auto memory, by contrast, stays per-repo in `~/.claude/projects/<project>/memory/` [CCM].
- **Codex:** the same instruction in `~/.codex/AGENTS.md`. Unverified: the exact global path in the current Codex version.
- **pi:** zosmaai/pi-llm-wiki implements the pattern natively (`wiki_retro`, `wiki_observe`, `wiki_recall`) [PI].
- **nerdr-app / hc:** hc runs every agent, so a future nerdr feature could trigger the end-of-session inbox write. This is speculative, not researched.

## 5. Alternatives and neighbours

| Option | What | Choose when |
|---|---|---|
| Claude Code `CLAUDE.md` + auto memory | Human-written rules, plus agent-written notes per repo (`MEMORY.md` index; first 200 lines or 25 KB loaded every session) [CCM] | Knowledge is project-specific and only Claude needs it. No setup. |
| `AGENTS.md` in each repo | Cross-agent project instructions | Rules for working in one repo (build, style, constraints). |
| RAG / NotebookLM / vector DB | Retrieves chunks at query time; nothing accumulates [G] | A large or changing corpus (thousands or millions of docs), or one-off questions. VB puts the LLM Wiki's ideal range at 100–10k high-signal docs [VB]. |
| qmd alone | Local hybrid search over existing Markdown [QMD] | You want search over the existing Obsidian vault without the LLM rewriting anything. |
| basic-memory (MCP) | Agent memory stored as Markdown, via an MCP server | https://github.com/basicmachines-co/basic-memory. Unverified: features not reviewed. |
| agentmemory (rohitg00) | Persistent memory for coding agents (author of LLM Wiki v2) | https://github.com/rohitg00/agentmemory (30k stars). Unverified: features not reviewed. |
| Transcript-to-wiki tools (pratiyush/llm-wiki, llm-wiki-compiler `ingest-session`) | Build pages from saved agent sessions after the fact | You want to mine past sessions rather than change agent behaviour. |
| Fine-tuning on the wiki | Karpathy's stated long-term direction: the wiki becomes training data [VB] | Not practical for personal use now. |

---

## Copyable setup

### Folder tree

```text
~/git/wiki/                  # git repo; open as its own Obsidian vault
├── AGENTS.md                # schema (canonical)
├── CLAUDE.md                # contains only: @AGENTS.md
├── index.md                 # catalog (regenerated or LLM-maintained)
├── log.md                   # append-only
├── inbox/                   # agent drop-zone, one file per session, never edited after write
├── raw/                     # immutable sources
│   ├── articles/            # Web Clipper output
│   ├── papers/
│   ├── sessions/            # processed inbox files
│   └── assets/              # downloaded images
├── wiki/
│   ├── overview.md          # current synthesis
│   ├── entities/            # tools, projects, people, orgs
│   ├── concepts/            # ideas, patterns
│   ├── sources/             # one summary per raw source
│   ├── howto/               # procedures, gotchas
│   ├── decisions/           # decision + why + date
│   └── answers/             # filed-back query results
└── scripts/
    ├── lint.sh              # broken [[links]], orphans, pages missing sources:
    └── index.sh             # optional: rebuild index.md from frontmatter
```

Setup commands:

```sh
mkdir -p ~/git/wiki/{inbox,raw/{articles,papers,sessions,assets},wiki/{entities,concepts,sources,howto,decisions,answers},scripts}
cd ~/git/wiki && git init
printf '@AGENTS.md\n' > CLAUDE.md
printf '# Index\n' > index.md; printf '# Log\n' > log.md
# optional search: npm i -g @tobilu/qmd   (unverified package name; see https://github.com/tobi/qmd)
#                 qmd collection add ~/git/wiki/wiki --name wiki && qmd embed
```

In Obsidian:
- Open `~/git/wiki` as a vault.
- Set Files and links → Attachment folder to `raw/assets`.
- Set "Download attachments for current file" to a hotkey.
- Install the Dataview plugin. [G]

### Starter `AGENTS.md`

```markdown
# Wiki schema

This repo is an LLM-maintained knowledge wiki (Karpathy "LLM Wiki" pattern).
Humans curate sources and ask questions; agents write and maintain `wiki/`.

## Layout
- `raw/`: immutable sources. Read, never edit, never delete.
- `inbox/`: drop-zone. Agents add new files here; never edit an existing inbox file.
- `wiki/`: pages. Subfolders: entities, concepts, sources, howto, decisions, answers. `wiki/overview.md` is the current synthesis.
- `index.md`: one line per page: `- [[path/page]]: one-line summary`. Grouped by folder.
- `log.md`: append-only. Entry header: `## [YYYY-MM-DD] <op> | <title> | <agent>`.

## Page format
- File name: kebab-case, singular (`wiki/entities/herdr-core.md`).
- Frontmatter:
  ---
  type: entity|concept|source|howto|decision|answer
  sources: [raw/..., https://...]
  updated: YYYY-MM-DD
  updated_by: claude|codex|pi|human
  ---
- Body: summary first (≤3 lines), then facts as bullets, each with a source link.
- Sections when relevant: `## Contradictions / superseded` (never delete old claims; mark them superseded with date and source), `## Open questions`.
- Link with `[[wikilinks]]`. Every page has at least one inbound link (from index or another page).

## Rules
1. Extend, don't create: before making a page, search `index.md` and `rg -i '<term>' wiki/`. Prefer adding to an existing page.
2. Re-read a page immediately before editing; edit sections, never rewrite whole files.
3. No claim without a source. Quote numbers exactly as in the source.
4. Contradictions are recorded on both pages, not resolved silently.
5. One git commit per operation: `<op>: <title>`. Run `git status` first; don't commit unrelated changes; never force-push or rebase.
6. Only one integrating operation (ingest, inbox, lint) at a time: create `.wiki.lock` with your agent name and time; if it exists and is <1h old, stop and report. Delete it when done.

## Operations
### capture (any agent, during or after other work; no lock needed)
Write `inbox/YYYY-MM-DD-HHMM-<agent>-<slug>.md` with: context (repo, task), facts learned, decisions + why, sources (paths, commits, URLs). Commit `inbox: <agent> <slug>`.

### ingest <source | inbox>
1. Take lock. 2. Read the source (for inbox: each file, oldest first).
3. Tell the human the key takeaways (skip if running unattended).
4. Write/update `wiki/sources/<slug>.md` (for raw sources).
5. Update related entity/concept/howto/decision pages and `wiki/overview.md`.
6. Update `index.md`; append to `log.md`.
7. Inbox files: `git mv` to `raw/sessions/`. 8. Commit, release lock.

### query <question>
Read `index.md`, then relevant pages (use `rg` or `qmd query` if the index isn't enough), drill into `raw/` when pages lack detail. Answer with `[[page]]` citations. If the answer is reusable, save it as `wiki/answers/<slug>.md`, add to index, log it.

### lint
Report then fix: broken links, orphans, pages without `sources:`, near-duplicate pages (merge them, leave a redirect line), contradictions, stale claims, concepts mentioned ≥3 times without a page, index entries without files and vice versa. Run `scripts/lint.sh` first if present. Log the result.
```

### Per-agent pointer (one line in each agent's global instructions)

```markdown
Cross-project knowledge wiki: ~/git/wiki (read its AGENTS.md before writing). At the end of a session that produced durable knowledge (tool facts, gotchas, decisions), run its `capture` operation. Never edit wiki/ pages outside an `ingest` run.
```

---

## Sources

| Key | Source | Date |
|---|---|---|
| G | Karpathy, "LLM Wiki" gist: https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f (raw text read in full) | 2026-04-04 |
| GC | Comments on the gist (same URL), read through WebFetch. These are claims by individual commenters, not verified. | 2026-10-01 to 2026-11-02 |
| VB | Carl Franzen, VentureBeat, "Karpathy shares 'LLM Knowledge Base' architecture that bypasses RAG…": https://venturebeat.com/data/karpathy-shares-llm-knowledge-base-architecture-that-bypasses-rag-with-an | 2026-04-03 |
| X | Karpathy tweet: https://x.com/karpathy/status/2039805659525644595. Not read directly. | Unverified: 2026-04-02 |
| V2 | rohitg00, "LLM Wiki v2": https://gist.github.com/rohitg00/2067ab416f7bbe447c1977edaaa681e2. Seen through summaries only. | 2026-04 |
| AH | Astro-Han/karpathy-llm-wiki README: https://github.com/Astro-Han/karpathy-llm-wiki | Repo created 2026-04-05, pushed 2026-07-23 |
| CO | AgriciDaniel/claude-obsidian README ("Nothing gets lost"): https://github.com/AgriciDaniel/claude-obsidian | Pushed 2026-10-25 |
| PI | zosmaai/pi-llm-wiki README (layers, guardrails, tools): https://github.com/zosmaai/pi-llm-wiki | Pushed 2026-11-03 |
| NV | nvk/llm-wiki README (session memory): https://github.com/nvk/llm-wiki | Pushed 2026-10-25 |
| NS | nashsu/llm_wiki README (serial ingest queue): https://github.com/nashsu/llm_wiki | Pushed 2026-10-21 |
| SA | SamurAIGPT/llm-wiki-agent README: https://github.com/SamurAIGPT/llm-wiki-agent | Pushed 2026-11-02 |
| AC | atomicstrata/llm-wiki-compiler README: https://github.com/atomicstrata/llm-wiki-compiler | Pushed 2026-10-30 |
| PR | pratiyush/llm-wiki README: https://github.com/pratiyush/llm-wiki | Read 2026-11-06 |
| QMD | tobi/qmd README: https://github.com/tobi/qmd | Pushed 2026-10-15 |
| CCM | Claude Code docs, "How Claude remembers your project": https://code.claude.com/docs/en/memory | Read 2026-11-06 |
| OB | Obsidian blog, "Free for work": https://obsidian.md/blog/free-for-work/ | 2025-02-20 |
