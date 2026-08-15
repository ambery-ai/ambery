# Harness Design

> Concept definition: see concepts.md §10 and its sub-concepts. This document fixes the data model, injection rules, trigger model, and JSONL storage format.

## Data Model

```rust
// Queue 输入条目（concepts §10c，输入串行化关口——只装输入）
struct QueueInput {
  role: 'system' | 'user',     // hook 内容 = system 输入；用户消息 = user 输入
  content: String,
  ts: i64,
}

// Context 消息（OpenAI Chat Completions 对齐，concepts §10b——完整消息数组）
struct ContextMessage {
  role: 'system' | 'user' | 'assistant' | 'tool',
  content: String | null,        // assistant 发起 tool_calls 时 content 可为 null
  tool_calls: ToolCall[] | null, // 仅 assistant
  tool_call_id: string | null,   // 仅 tool
  ts: i64,                       // epoch ms
}

// Filtered 内容（concepts §8/§11，Filter 后归一全文，agent 实际读到的终端内容）
// 不持久化——从 terminal-content.jsonl 原文 digest 现算（docs/storage.md §filtered_content 退役）
struct FilteredContent {
  instance: string,              // Code CLI 实例名
  filtered_content: string,      // Filter 后的 Terminal Content 全文
  source: 'hook' | 'timer' | 'fetch_terminal',
  ts: i64,
}

// Code CLI 实例清单（concepts §9/§13）
struct AgentEntry {
  hash: string,                  // sid8(session_id)；mock/扫描回退见 docs/hook.md §marker 定位
  name: string, project: string, // display 名 = <project>·<sid8>（即 tab 定位 marker）
  kind: string | null,           // CLI 种类（Filter 按它选择，docs/filter.md）
  tab: TabRef | null,            // tab 定位快照（session_end 的 closed 快照 tab 为 null）
  status: 'idle' | 'processing' | 'unknown' | 'closed',
  first_seen: i64, last_seen: i64,
}
```

## Queue Rules (concepts §10c)

1. **Queue holds inputs only**: hook content (system input), user messages (user input). assistant / tool output **does not go through Queue** and enters Context directly.
2. **Every input carries a source field**: source = the semantic reason that triggered this input (`user_chat` / `hook_stop_hint` / `hook_stop_content` / `hook_stop_report` / `hook_user_prompt` / `hook_notification` / `mock_hook` / `timer_scan` / `cron_tick`; for the complete set and enqueue points see `docs/concrete-insight.md §Queue 中的 System 消息来源`). Source is a first-class citizen driving per-source behavior such as effort level and priority; `release_one` passes it into `run_trigger`, and subsequent calls within the tool loop reuse it.
3. **Serial release + dual queue**: after each input is released, the entire round must finish (input written to Context → LLM → tool execution → output written to Context) before the next input is released — no parallel LLM calls. If an input arrives while processing is underway, it waits in the Queue. The Queue is split into two: `high_q` holds `user_chat` (the user directly asking pet), `normal_q` holds everything else; on release, if `high_q` is non-empty it releases from `high_q` first (FIFO), otherwise from `normal_q` (FIFO) — direct user questions to pet get priority, while each queue internally keeps its own arrival order.
4. The system prompt **is not** Queue input and not a Context message — it is the request header assembled fresh at each LLM call (base_prompt + AGENTS.md + system expression pool; user expression pool queried on demand); the content is stable and naturally cache-friendly, not persisted (for the head snapshot see docs/storage.md).
5. Hook trigger → AmberyBackend injects a `system` input into the Queue (e.g. "config-service finished; Context updated (4,958 chars). Evaluate whether to notify.").
6. **diff as events**: the bookkeeping events of instance registration / state flips do not go through Queue — they are silently attached via the Event Buffer (§10e); the LLM reconstructs the full picture from the event stream in Context, rather than injecting snapshots per round.

## Event Buffer Rules (concepts §10e)

- An independent input channel parallel to the Queue, storing Component interactions and silent bookkeeping.
- Each record carries a two-part payload: **natural language** (required, a description of the operation process) and a **structured state snapshot** (optional, attached only for todobox-like interactions).
- Structured snapshots for the same card within a single flush are **deduplicated and merged** into one final state (checking three todo items in a row → only one full items payload is sent).
- When the Queue releases an input: all Buffer entries (natural language + structured snapshot) are **merged with that input into one** `system` message entering Context, then cleared (attached semantics, no independent message).
- Never writes the `user` role; raw entries are not persisted (the merged message is written to the Context log).

## Context Rules (concepts §10b)

- Context = the complete message array (aligned with OpenAI messages): Queue release writes the input, LLM replies write assistant, tool executions write tool — the context source for LLM requests and also the persistent archive of the full conversation.
- Terminal content: Hook trigger → AmberyBackend reads Terminal Content → **raw text first stored in terminal-content.jsonl** → Filter → the normalized result updates the in-memory change-detection baseline; after release, what is injected into Context is the evaluation prompt (of the form "{name} finished; Context updated (N chars). Evaluate whether to notify.") — the normalized full text itself does not enter Queue/Context.
- The normalized full text **is not persisted**: the change-detection prev (each instance's last normalized full text) is kept in memory (lost on restart); "what exactly is that bug" type follow-up questions and the `fetch_terminal` fallback recompute the digest from the raw terminal-content.jsonl.
- Autonomy state records (type=autonomy) also write one per round into context.jsonl; when assembling the request, take the latest one and append it to the request end (concepts §4 / docs/storage.md).

## Compression (concepts §10d, auto-compact)

- Trigger (usage is the truth source): **the latest `usage.prompt_tokens` + the est increment of subsequently added messages >
  `effective_compression_limit()`** (the active profile's `context_window − reserve`;
  reserve defaults to the global `compression_reserve_default` 10K). **No context_window = no compression** (explicitly no guessing).
  The usage line is the authoritative truth (docs/storage.md §usage); est (chars/4, marked lossy) is only used when no truth is available
  (first round / restart without a usage line) and for incremental estimation, not as the primary source. No local BPE tokenizer is introduced
  (opencode / Claude Code both follow the API truth).
- Summary: generated by a **dedicated LLM call** (the configured model compresses history into one `system` summary; DebugAgent mode falls back to a deterministic stub, keeping tests deterministic).
- Shaking: keep original messages along complete turn boundaries; arbitrary per-message truncation is not allowed. One turn is the complete set from the release of one Queue input to the end of all LLM responses, tool calls, tool results, and the final reply it triggered.

### Config Fields

| Field | Default | Takes effect | Semantics |
|---|---:|---|---|
| `context_compression_keep_recent_messages` | 24 | cold | Retention target for original messages of completed historical turns; from the most recent completed turns backward, keep at least this many, truncating only between complete turns; the turn currently being processed is always kept complete, even if it exceeds this number. The assistant's tool_calls message and each tool result each count as one; a full tool interaction with the per-call cap of 10 calls is 11 messages, and 24 can retain about two batches of such interactions plus the final text reply, avoiding premature shaking away of the complete query truth. |

- **Reset to zero**: the diff baseline is cleared, all existing instances are treated as just discovered, and one diff enters as one system message — compression does not lose instance awareness.

## Memory (concepts §10f)

Memory is a **persistent understanding buffer** managed by Harness and actively maintained by the Agent: it replaces the Agent's reliance on the filesystem for recording understanding; it is not Context, not a compression summary, and not the terminal-content archive. It persists across turns, compression, and restarts; the backend, the user, and the Agent can all manage it, with the Agent adjusting it through `read_memory` / `write_memory`.

- Memory is one shared understanding for the whole of Ambery, not isolated per monitored project; it can record cross-project plans, working relationships, and user collaboration preferences.
- Its persistent form is one Memory Workspace (notes/ long-term understanding + cards/ durable work artifacts): for the directory structure, indexes, write rules, and read-only contract see docs/memory.md.

For the two tools' parameter schemas, validation, return structures, and the index/AGENTS.md generation contract, see **docs/memory.md** (finalized).

#### ⟡ Consistency Analysis

Memory Workspace, Cron, and Card are all cross-restart concepts managed by Harness: each is restored from its persistent carrier into a runtime projection, managed through controlled entries by the user, the backend, or the Agent, and observable. They do not belong to Context, Queue, or Event Buffer, and the truth must not be delegated to the View or to the LLM's local state. Their persistent forms may differ by semantics — notes / Card use files, Cron uses an append-only schedule log — what is consistent is ownership, restoration, and consumption boundaries, not forcing one file format.

## Cron (concepts §10g)

Cron is Harness-managed **persistent scheduling and delayed dispatch**: it records future work, e.g. sending a daily report prompt every evening; it can also support continuing a planned behavior after a short sleep, e.g. waiting a few seconds before executing `set_autonomy`. It persists across restarts and can be managed by the backend, the user, and the Agent.

The Agent adjusts Cron via `cron_create` / `cron_delete` and requests a short wait via `sleep`; under the hood the three use the same Harness scheduling implementation (`CronScheduler`). For task representation, cron.jsonl format, due-time behavior, and the three tools' parameters/validation/returns, see **docs/cron.md**.

## Storage (concepts §13, spec: JSONL)

For layout, per-file semantics, and record formats see **docs/storage.md**. Key points:

- Two domains: Config (`config.json` + identity prompt `AGENTS.md`) at the root; Storage (`queue` / `terminal-content` / `context` / `work-agents` / Memory / Cron) under `storage/`. The read-only `AGENTS.md` inside the Memory root is not the same file as the Config-domain identity prompt.
- **Context is the memory view**: the full-fidelity log is in context.jsonl (a unified envelope for message / autonomy / head / filtered_content / compression boundary) — compression is a marker, not deletion, and the OpenAI context is almost completely reconstructable.
- **Queue input traces**: append-only queue.jsonl (enqueue records; after release the original text is already in the Context message line, so queue.jsonl is the queuing trace, not the conversation body).
- Event Buffer raw entries are not persisted: scratch-area semantics, crash loss acceptable (the merged system message is written to the Context log).

## Trigger Model (One Complete Turn for One Queue Input)

```
Queue 放行一条输入（附带 Event Buffer 合并为一条）
  → Context 写输入
  → 现拼 system prompt 请求头（base_prompt + AGENTS.md + 系统表情池，不落 Context）
  → Compression 检查（Context 超阈值 → 专项摘要 + shaking + 归零重 diff）
  → 附加 Autonomy 状态到请求末端
  → call LLM（请求 = 请求头 + Context 全部消息 + Autonomy 状态）
  → loop: assistant tool_calls → AmberyBackend 执行 → tool result 追加 Context → 再做 Compression 检查 → 再 call
  → assistant content 追加 Context
  → Queue 放行下一条
```

## Frontend-Backend Protocol

Tauri mode: the frontend communicates with core via **Tauri native IPC** (`#[tauri::command]` + `invoke()` + `app_handle.emit()` + `listen()`).
Only external hook scripts keep HTTP `POST /hook` (cross-process calls from outside the process, where a Tauri command is unreachable).

Browser debug mode: core runs as the full router host with `ambery-case serve` (docs/core-server.md), and the frontend connects directly through a thin HTTP+WS loopback.
