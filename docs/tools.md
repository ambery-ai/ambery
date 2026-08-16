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

- `context.jsonl` — Context messages (message / autonomy / head / Compression boundary)
- `queue.jsonl` — Queue input enqueue records
- `effect.jsonl` — action stream (render / close / window lifecycle / event_emit)
- `terminal-content.jsonl` — Terminal Content raw text

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
| `→` / `l` | expand the focused foldable (trajectory form); open the detail pane on a leaf (trajectory event) or any flat row |
| `gg` | jump to the top |
| `G` | jump to the bottom |
| `Tab` | switch file source |
| `/` | start filtering (kind / summary substring) |
| `f` | toggle follow (tail newly written records) |
| `q` / `Esc` | quit |

### Detail pane

A right-hand pane (40 % width) shows the untruncated content of the focused row: its list line as a header, then the full text. `→` / `l` on a leaf row (an event in the trajectory form; any row in the flat form) focuses the pane, where `↑` / `k` and `↓` / `j` scroll the content; `←` / `h` (or `Esc`) returns to the list. The row model keeps the full untruncated content (`detail`), while the list always renders the truncated summary.

### Trajectory form (`--trajectory`)

References the trajectory concept from dsh: projects the flat JSONL into a **turn-aware compact trajectory ledger** — preserving causal structure rather than just giving line-by-line logs.

- A `context.jsonl` `session` line = session boundary (heavier rule); each `queue.jsonl` line = one turn boundary (one Queue release round = one trigger); remaining lines are attributed to the nearest turn by ts and indented as event lines.
- When there is no queue data (common in case snapshots), a user message in `context.jsonl` degrades into a turn boundary.
- **Per-item folding**: folding is per session / per turn, not global. Press `←` / `h` to collapse the rows under the focused foldable — on a Session row its turns and events, on a Turn row its events, on an event row its containing turn; `→` / `l` expands it back (on a leaf it opens the detail pane instead). Fold state is keyed by stable row identity (session ordinal / turn index), so rows fold and unfold independently; a folded row shows a `[+n]` marker with the hidden count. `/` filters, Tab switches files, `f` follows, same as the normal form.

### Implementation

Rust standalone bin, reusing core's JSONL record types (`ContextMessage` / `Effect` etc.). The directory parameter defaults to `storage_dir` (overridable by `AMBERY_STORAGE_DIR`), and an explicit directory can also be passed.
