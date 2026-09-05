# Storage Design

English | [中文](storage.zh.md)

> See concepts.md §7/§8 for the conceptual definitions. This document defines the directory layout, per-file semantics, record formats, and lifecycle.

## Layout: Two Domains

```
%USERPROFILE%\.config\ambery\   (AMBERY_CONFIG_DIR can override; resolved by core/paths.rs)
  config.json              # Config domain: launch config (concepts §7)
  AGENTS.md                # Config domain: pet identity prompt — config in nature, not session data
  storage/                 # Storage domain: world state + context logs (AMBERY_STORAGE_DIR can override, concepts §8)
    queue.jsonl            # Queue input queueing records (concepts §4c-1)
    terminal-content.jsonl # Terminal Content raw-text archive (before Filter)
    context.jsonl          # unified full-fidelity log: conversation + Autonomy + request head snapshot + normalized full text + Compression boundary
    work-agents.jsonl      # instance lifecycle permanent event log
    effect.jsonl           # frontend/backend unified action stream (Effect: backend side effects + non-read-only Tauri runtime actions)
    memory/                # Memory Workspace: long-term understanding + durable work artifacts
      AGENTS.md            # read-only: workspace navigation information
      index.md             # read-only: auto-summarizes names and descriptions in notes/
      notes/               # Agent's long-term understanding (no further directory subdivision for now)
        *.md               # short, fragmented ordinary notes
      cards/               # durable Component / work artifacts
        <id>.card.json     # a file is a Card: complete JSON, content + Surface intent + spatial layout
    cron.jsonl             # Harness's persistent schedule and delayed scheduling
```

**Philosophy** (same as the Claude Code session format): **logs are sacred, views are ephemeral**. Everything goes into files, append-only, never rewritten; the request Context is merely a projection of the log. Goal: **the OpenAI API context can be almost fully reconstructed** — conversation (including tool_calls/reasoning_content), events, autonomy, instances, and terminal content are all traced; Compression is a marker, not deletion; assembled request heads are snapshotted too — more complete than CC (CC does not version CLAUDE.md).

## config.json (Config Domain)

concepts §7. Launch config: timer parameters, Compression thresholds and retention targets, system/user expression pools, LLM profiles + active selector, view_scale, set_autonomy_default_ttl_ms, stop_hook_mode, theme/themes, ui_language/harness_language, name, tool call budget. (The hook port is not a Config field: default 127.0.0.1:47600, `AMBERY_PORT` explicitly overrides it — changing the port requires syncing the hook config; docs/core-server.md §Port semantics.)

- Write: bootstrap writes defaults / the unified Config modification entry writes back. Read: loaded at startup + auto-reload of the external file while running.
- The key itself lives only in the environment (the provider's `api_key_env`), never in the file. **App-level env layer**: `env` (0600, `KEY=value` lines) is an app-level environment-variable layer that *overrides* the system environment — resolution order is env file → process environment (first hit wins). The env file is the in-app key store (the setup modal writes here); it is not part of `config.json` and never contains the Config domain's data. See docs/llm-setup.md §Key storage model.

## AGENTS.md (Config Domain)

The pet identity prompt, concatenated with base_prompt into the **request head assembled fresh each turn** (not written to Context).

- bootstrap: if it does not exist, write the built-in default (once only); the user may edit it by hand.
- **Read fresh at each LLM request assembly** (hot effect: the next request uses it after editing); if it cannot be read while running, keep the already-loaded live content and show the load error in the reflection Config UI; do not overwrite it with the default content.
- Rationale for placing it in the Config domain: stable, editable, unchanged across sessions — same category as config.json; one stores parameters, the other stores identity.

## terminal-content.jsonl (Raw Archive, Before Filter)

Terminal **raw text** (ANSI/spinner all included; the instantaneous full text of the terminal session, concepts §5a Source content). One line is written per read:

```json
{"instance":"demo-webapp","raw":"…raw text…","source":"hook","ts":1784952913010}
```

- `source`: `hook | timer | fetch_terminal`.
- Write timing: on every read, **write the raw text first, then filter** — even a crash in between leaves at least the raw text.
- Role: ground-truth archive. Replay the raw text to validate when Filter rules iterate; debug "what Filter filtered out". **Normally not read, not replayed at startup**.

## context.jsonl (Unified Full-Fidelity Log)

**One file holds everything needed to reconstruct the OpenAI context** (same style as CC's single file + type discrimination), append-only, never rewritten. Each line is a unified envelope `{type, ts, ...}`:

| type | payload | goes into request? |
|---|---|---|
| `message` | ContextMessage full fidelity: {role, content, tool_calls, tool_call_id, reasoning_content} | ✓ Context body |
| `autonomy` | {content: `[face: key, motion: key]`}, one per turn (whether changed or not) | ✓ latest line appended at the end of the request |
| `head` | {content: the assembled request head}, **written only on change** (diff) | ✓ latest line serves as the request head |
| `usage` | {prompt_tokens, completion_tokens} — the ground truth of each LLM call (cache breakdown is constantly 0 and not stored) | ✓ latest line is the Compression ground-truth baseline |
| `compact_boundary` | {summary, pre_tokens, post_tokens, duration_ms} | view projection marker |
| `session` | {sessionId}, one per startup | session boundary |

> **`filtered_content` line type**: the normalized full text is **not persisted** — it can be recomputed by digesting the raw text in terminal-content.jsonl, so persisting it would be redundant. `content` lines in old files are ignored during replay. The prev used for change detection (the last normalized full text per instance) is kept in **memory** (updated after scan; lost on restart: the first scan after restart necessarily reports a change once — an accepted cost); `fetch_terminal` fallback/follow-up is computed on demand from the raw text; observe's `filtered_content` item is likewise computed on demand (docs/case-runner.md §Observability system).

- **`message`**: every Context append (Queue-released input + LLM assistant/tool output) is synchronously written as one line — the conversation is fully faithful, with assistant tool_calls and reasoning_content recorded verbatim (a hard requirement for replaying thinking models; the chain of thought in plain-text replies is also fully faithful; recording ≠ replay: in replay, only tool_calls messages carry reasoning, see docs/agent-loop.md).
- **`autonomy`**: one per turn as defined in concepts §1a; the latest is taken at assembly.
- **`head`**: the assembled result of base_prompt + AGENTS.md + the system expression pool, **written only on change** — request-head history is also reconstructible (design decision); the user expression pool is not automatically injected into the request head, but queried on demand via `edit_config`.
- **`usage`**: the **single authoritative source** for token accounting. Every LLM call (including each round of the tool loop and Compression summary calls) writes one line; the read semantics is **override** — the latest line's `prompt_tokens` is exactly "the precise token count of the last full request body (head+messages+autonomy)" (opencode is isomorphic: step-level tracing, the latest value represents current context occupancy). The cache breakdown is measured as constantly 0 and is not stored.
- **`compact_boundary`**: Compression is a marker, not deletion — the in-memory view shakes, files are fully retained, and Compression is auditable (both summary and original text are present).
- **`session`**: one line per startup. Reconstructing a run = slicing between two session markers.

### View Reconstruction (OpenAI Context Projection Rules)

1. Take the target session interval (default: the latest one).
2. The last `compact_boundary` in the interval: its summary becomes the first system message + the `message` lines after it become the Context; if there is no boundary, take all `message` lines.
3. The latest `head` at the interval's end time serves as the request head; `autonomy` is computed on demand at request assembly (freshly read from agents/config, no replay index) — the projection rules need not keep an index for it (docs/window-follow.md has the same consistency analysis: effect only audits actions and never infers the current state from them).

= the complete request at that moment; any historical moment follows the same rule (slice with the then-current head / autonomy / boundary).

## work-agents.jsonl (Permanent Instance-Lifecycle Event Log)

**Permanent record**: no folding, no rotation, no cleanup. One hash per agent per lifecycle (names repeat — a tab closed yesterday and a same-named tab opened today are two lives):

```json
{"hash":"a1b2c3d4","name":"demo-webapp","project":"nap","status":"processing","ts":1784952913010}
{"hash":"a1b2c3d4","name":"demo-webapp","project":"nap","status":"idle","ts":1784953125000}
{"hash":"e5f6a7b8","name":"demo-webapp","project":"nap","status":"processing","ts":1785050000000}
```

- Each line = a **complete snapshot** after a status change (self-contained; no need to find the registration line when reading the log).
- Fields (docs/agents/claude/hook.md): `{hash, name, project, kind, status, tab, first_seen, last_seen}`
  - `hash` = **sid8(session_id)** — the first 8 characters of session_id (same name, different life: reopening the same project = a new lifecycle; same origin as docs/agents/claude/hook.md §marker location); mock hook without session_id falls back to `short_hash(name + project + first_seen)`.
  - `name` = `<project>·<sid8>`; the display name **is also the tab-location marker** (one name, two uses).
  - `kind` = CLI kind (`"claude"`, input to the per-instance access-side filter strategy, docs/agents/filter.md).
  - `tab` = `{hwnd, index}` location result. **Same treatment as status: just one field of the snapshot** — event snapshots after a successful location carry it; "re-finding" = appending another new snapshot; the current value is always derived by projection (the latest line per hash); no in-place updates. The closed snapshot of session_end has tab = null.
  - `first_seen` / `last_seen` = the moment the backend first saw / most recently saw the event (the backend only knows when it saw something).
- Instance status (`idle | processing | unknown | closed`) is a belief maintained from evidence — Ambery observes processes it does not control, never assuming its lifecycle knowledge is complete. Belief moves only on concrete evidence: a hook event, a terminal read, or a process check. `closed` is terminal — out of the active set — reached on confirmed close evidence (SessionEnd hook; or a positively confirmed "gone": reader NotFound / process check) or by retiring a long-unknown instance (lost track of; the confirmed-dead vs lost distinction is not maintained); the patrol scan is the observation cycle that delivers the evidence, never a time inference to death. A transient read failure is never death. A permanent log must still retire unknown instances, otherwise the panorama accumulates corpses forever.
- **Registry (current state) = log projection**: replay folds by hash and takes the latest.
- The panorama after startup zero-resync = the set in the projection where `status ≠ closed`; unknown entries are shown as unconfirmed, not as alive.

## Context: In-Memory View, Log in context.jsonl (concepts §4b)

Context (the complete message array) is the **in-memory projection** of context.jsonl (see the view reconstruction rules above). At runtime, appends are double-written: in-memory Context + the `message` line in context.jsonl.

- Restart: write a `session` marker → empty Context + **startup zero-resync** (one panorama system message of surviving instances, likewise written as a `message` line) — the same mechanism as Compression's zero-reset re-diff (docs/harness.md).
- No resume by default; history is fully on file, and `--resume` is simply the application of the projection rules (design decision).
- The Event Buffer writes the merged system message attached to the released input as a `message` line (raw entries are not stored; staging-area semantics; loss on crash is acceptable).

## Queue: Input Queuer, Log queue.jsonl (concepts §4c-1)

Queue holds pending inputs (hook content, user messages) and releases them serially (after release, the text enters Context's `message` line). The append-only queue.jsonl records each enqueued input line by line — it is the **queueing trajectory**, not the conversation itself.

- Losing unreleased inputs on crash is acceptable: hooks are transient signals (Terminal Content is still on screen and can be re-read), and user messages are re-sent by the panel.
- The legacy queue.jsonl (message-log-era semantics) and the pre-rename agents.jsonl are not migrated (design decision).

## Memory Workspace (Harness Persistent Workspace)

See concepts §4d / docs/harness.md for the concept and read/write boundaries. `storage/memory/` is the single Memory Workspace root; flatness is not required:

- `notes/`: the Agent's long-term understanding; an ordinary note is an `.md` file subject to a length cap, and directories are no longer subdivided for now. `index.md` automatically summarizes, in table form, the names of notes and the description that is mandatory on every write.
- `cards/`: durable Components / work artifacts; one `<id>.card.json` file is one Card. The file is complete JSON, with its Component content, Surface intent, and spatial layout co-located (see docs/components.md §Card file for the file contract).
- `AGENTS.md`: navigation information for the entire workspace; `index.md` and `AGENTS.md` are read-only by default. They are not the `AGENTS.md` used as the system prompt in the Config domain.

Notes may reference Cards by the stable relative path `cards/<id>.card.json`; a Card is not an ordinary note, does not participate in the note index, and is not managed through `read_memory` / `write_memory`. The Memory Workspace survives restarts and serves Ambery's cross-project understanding and work artifacts; it is not a copy of Context, Compression summaries, or Terminal Content.

#### ⟡ Consistency Analysis

The Card file is the truth of the current ongoing work artifacts; its complete JSON co-locates Component content, Surface intent, and spatial layout. It does not need to fold from the last line like `cards.jsonl` to derive the current state. `effect.jsonl`, by contrast, only answers "what actions happened": it is not replayed, does not carry Card truth, and cannot infer which Cards still exist from window opened / closed events. File-as-Card gives work artifacts a stable address that Memory notes can reference, while action auditing remains append-only.

## Timer (Harness Persistent Schedule and Delayed Scheduling)

See concepts §4e / docs/harness.md for the concept and boundaries. `cron.jsonl` persists future schedules and delayed scheduling, and is restored by replay folding after restart; the backend, the user, and the Agent can all manage it. `cron_create` / `cron_delete` and `sleep` share the same underlying scheduling implementation (waiters are not persisted). See **docs/cron.md** for the append-only event line format (create / fire / delete) and folding rules.

## effect.jsonl (Frontend/Backend Unified Action Stream)

Append-only log of the Effect action stream (docs/case-runner.md §Observability system / case-eval-system.md): **actions are recorded, not driven**. Both backend side effects and frontend UI/runtime actions are uniformly recorded — including UI actions the frontend performed (rendering a bubble, showing a banner). Effects do **not** drive rendering: the frontend renders by its own local logic (optimistic bubbles, self-detected banners) and records the action afterward through the frontend reporting channel. Tauri runtime actions cover the WebView's `@tauri-apps/api` and the Rust shell's `tauri` API, both recorded through the runtime action layer after success.

```json
{"type":"effect","origin":"backend","kind":"render_component","payload":{...},"ts":1785600000000}
{"type":"effect","origin":"frontend","kind":"window_moved","payload":{"x":100,"y":200},"ts":1785600001000}
{"type":"effect","origin":"frontend","kind":"error_bubble","payload":{"message":"..."},"ts":1785600002000}
```

- `origin`: frontend / backend (initiator)
- `kind`: action type — backend (render_component / close_component / set_autonomy / config_changed / assistant_delta / assistant_done / llm_error); frontend (user_message / user_bubble / error_bubble / setup_banner / interaction / config_update / window_opened / window_closed / window_resized / window_moved / window_drag / window_visible / window_hidden / event_emit)
- `payload`: payload (snake_case field names, a Storage convention; unrelated to the camelCase form delivered over WS)
- High-frequency events (onMoved dragging) are packed into one line
- observe's `effects` item (path category) reads it

**Recording points (backend, one recording point per variant to prevent double writes)**:

| Variant | Recording point | Coverage |
|------|--------|------|
| config_changed | `edit_config_update` finish (LLM tool path, origin=backend) | LLM edit_config |
| config_update | endpoint finish (server post_config / Tauri set_config, origin=frontend) | frontend settings panel |
| render_component / close_component / set_autonomy | `execute_tool` finish (includes early returns; single-point recording via inner wrapper) | LLM tool loop / case tool_call step / tests |
| assistant_delta / assistant_done / llm_error | the sink call site in `run_trigger` | streaming and non-streaming finish / LLM failure degraded |
| user_message | `enqueue` (when role==User, origin=frontend) | post_user / append_user / case user step |
| user_bubble / error_bubble / setup_banner | frontend `reportEffect` (record_effect command / POST /effect, origin=frontend) | frontend rendered a user bubble / error bubble / setup banner |
| interaction | endpoint (server post_event / Tauri push_event, origin=frontend) | frontend Component interaction |
| window_* / event_emit | Tauri runtime action layer (WebView `record_effect` command / POST `/effect`; the Rust shell uses the same recording entry) | non-read-only Tauri runtime actions of WebView / Rust shell (docs/effect-reporting.md) |

Recording is best-effort and does not block the main flow (consistent with effect_sink semantics); `Effect::effect_kind_payload` is an **exhaustive match** over kind/payload — adding a new Effect variant causes a compile error here (compile-time enforcement into the action stream).

## Startup Replay Flow

1. Fold `work-agents.jsonl` line by line → registry projection.
2. Build the index over `context.jsonl` in a streaming fashion: latest `head`, latest `autonomy` (the normalized full text is not replayed — there is no persistent archive; `terminal-content.jsonl` is not replayed).
3. Write a `session` marker; empty Context + zero-resync: one panorama system message (surviving instances), written as a `message` line.
4. `AGENTS.md` does not exist → bootstrap writes the default.
