# Tools Design

Unified definition of development tools: script tools under the `tools/` directory + core's standalone bin tools.

## Tool list

| Tool | Description | Invocation |
|---|---|---|
| `locate.ps1` | enumerates all windows under a process (locating/verifying window-related changes) | run directly |
| `run-vite-dev.ps1` | vite dev resident runner (auto-restart on crash) | run in background as a resident process |
| `ambery-activity` | activity viewer that reads Storage JSONL (core standalone bin) | see below |

## ambery-activity — Storage activity viewer

Reads the JSONL files under the Storage directory (docs/storage.md) and interactively inspects the internal message flow in a TUI. Used during development/debugging to observe what the system actually writes.

### Data source

Reads according to the storage layout (docs/storage.md):

- `queue.jsonl` — Queue input enqueue records (one turn boundary per line)
- `context.jsonl` — Context messages (message / autonomy / head / session / Compression boundary)
- `effect.jsonl` — action stream (render / close / window lifecycle / event_emit)
- `terminal-content.jsonl` — Terminal Content raw text
- `work-agents.jsonl` — Code CLI instance lifecycle records
- `cron.jsonl` — scheduled tasks

### Form

TUI interactive interface (`ratatui`). Core interactions:

- **File switching**: switch between different JSONL files
- **Scrolling**: page up/down through historical records
- **Filtering**: filter by kind / role / source
- **Follow** (`--follow`): tail newly written records

### Keybindings

Both forms share the same key set; horizontal movement mirrors vertical:

| Key | Action |
|---|---|
| `↑` / `k`, `↓` / `j` | move cursor up / down (scroll the detail pane when focused) |
| `←` / `h` | collapse the focused foldable (trajectory form); return to the list from the detail pane |
| `→` / `l` | expand a folded container, or descend into an expanded one (trajectory form); open the detail pane on a leaf (trajectory event) or any flat row |
| `gg` | jump to the top |
| `G` | jump to the bottom |
| `Tab` / `Shift+Tab` | switch file source forward / backward |
| `/` | start filtering (kind / summary substring) |
| `f` | toggle follow (tail newly written records) |
| `q` / `Esc` | quit |

### Symbol prefixes

Every row carries a one-symbol prefix (single glyph + one space) identifying its source file at a glance:

| Symbol | Source |
|---|---|
| `▸` | turn boundary / `[pre turn]` region (foldable container) |
| `·` | `context.jsonl` (message / autonomy / head / usage / session / compact_boundary) |
| `▪` | `effect.jsonl` |
| `–` | `terminal-content.jsonl` |
| `◇` | `work-agents.jsonl` (Code CLI instances — supervised external agents, not this system's LLM) |
| `◷` | `cron.jsonl` |

The selected row is marked with `❯` (not `▶`, which would collide with the `▸` container prefix). Source is expressed solely by the symbol — rows carry no redundant `[file]` tag; the detail pane still shows the full metadata.

### Detail pane

A leaf row has two states — opened and closed. `→` / `l` on a leaf row (an event in the trajectory form; any row in the flat form) opens a right-hand pane (40 % width) with that row's untruncated content: its list line as a header, then the full text. While open, `↑` / `k` and `↓` / `j` scroll the content; `i` toggles fullscreen (the pane fills the whole area, `i` or `Esc` exits back to the split view); `←` / `h` (or `Esc`) closes the pane and the list returns to full width. The row model keeps the full untruncated content (`detail`), while the list always renders the truncated summary.

### Trajectory form (`--trajectory`)

Projects the flat JSONL into a **turn-centric trajectory ledger**: the top-level unit is one complete processing round of this system's LLM — a **turn** = one Queue release (concepts §10c; one `queue.jsonl` line per turn). Everything the round produced — context writes, effects, terminal reads, agent records, cron actions — is attributed to the nearest turn by ts and rendered one level indented under it.

- Each `queue.jsonl` line = one turn boundary. When there is no queue data (common in case snapshots), a `context.jsonl` user message degrades into a turn boundary.
- **Code CLI is not this system's LLM**: supervised external instances (concepts §9) appear only as ordinary `◇` rows, never as hierarchy levels.
- A `context.jsonl` `session` line is an ordinary `·` row (a context-store startup boundary, one per backend startup) — attributed to its turn, not a container.
- Rows before the first turn render under a `[pre turn]` region — the same glyph and folding semantics as a turn.
- **Folding** is per container, two levels only (turn / `[pre turn]` > its rows): `←` / `h` collapses the focused container — on a container row itself, or on any of its rows (folding up to the containing container); `→` / `l` expands a folded container, or descends into an expanded one (cursor to its first row). A folded container keeps its boundary row with a `[+n]` marker (hidden count). `/` filters, Tab / Shift+Tab switch files, `f` follows, same as the normal form.

### Implementation

Rust standalone bin, reusing core's JSONL record types (`ContextMessage` / `Effect` etc.). The directory parameter defaults to `storage_dir` (overridable by `AMBERY_STORAGE_DIR`), and an explicit directory can also be passed.
