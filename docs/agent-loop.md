# Agent Loop Design

> Concept definitions: see concepts.md §2/§9b/§10a. This document fixes the LLM abstraction, the Tool Set protocol, and the mock hook contract.


## Principles

> **No code logic that need not exist** — after the budget is exhausted, still use the existing tool result → LLM closing chain: make one final text-reply request normally with empty tools; do not inject an extra terminating system record and do not automatically start a new turn.
> **Scope of this document** — this document defines the trigger lifecycle, LLM requests, tool-call budgets, and post-exhaustion closing; for Config migration, map reconcile, and descriptor mechanisms, see `docs/config.md`.

## LLM Abstraction

```rust
trait Llm {
    async fn complete(&self, messages: &[ContextMessage], tools: &[ToolDef]) -> LlmOutput;
}
struct LlmOutput { content: Option<String>, tool_calls: Vec<ToolCall> }
```

Implementation and assembly (`LlmBackend::from_config`):
- `active: "debug"` → `DebugAgent`: pure mock, zero logic. Hand the entire Context to an external decision source (silence/script/HTTP brain) and returns verbatim the `LlmOutput` that the decision source gives (docs/debug-agent.md).
- `active: "<provider 名>"` → `OpenAiClient` (OpenAI-compatible endpoint); **on failure, fall back to DebugAgent** (initialization failure: env unset / provider nonexistent → overall fallback; call failure: HTTP/timeout/parse → fallback for that turn). Failures are no longer silent: while falling back, also emit an `llm_error` effect, and pet renders an "LLM call failed" error-frame Card (updated in place under the same id `llm-error`).

**Config v2 (multi-profile + active selector)**: providers stores each provider's `base_url/model/api_key_env/temperature`; switching only changes `active` without losing config; keys live only in environment variables. On first startup, when config.json does not exist, write the default presets (deepseek/moonshot/zhipu/openai/ollama public providers); private providers are added by the user to the local config.json.

**reasoning_content return requirement for thinking models**: when a thinking model has thinking mode enabled, any assistant message with `tool_calls` in the history **must carry the `reasoning_content` field back**, otherwise 400 (empty string passes; `thinking:{type:"disabled"}` is not accepted). Therefore ContextMessage persists `reasoning_content` (old records lack this field; replay pads an empty string), and AmberyBackend stores the chain of thought on every turn's assistant tool_calls message.

**Record ≠ replay**: the reasoning_content of plain-text replies is **also stored to disk at full fidelity** (context.jsonl is auditable; debug/case can inspect the chain of thought), but it is **not replayed** — DeepSeek officially requires not returning reasoning across turns (each turn's reasoning is one-shot), and build_body writes reasoning_content only in the tool_calls branch, so plain-text replies cost no tokens and carry no 400 risk.

## Tool Set Design Principles

> **No special rules; let semantics define behavior** — tool parameters are fully defined by schema; implicit conventions are forbidden (missing parameter switches mode, empty value triggers query, etc.). The single source of implementation is `tool_set()` in `core/src/llm.rs`.

> **Progressive disclosure; query on demand** — Config is deeply nested; the LLM discovers paths and types layer by layer through tool call-feedback, not by relying on injected external Schema.

## Tool Set Protocol (concepts §10a)

Nine function definitions, CLI-style names; after execution AmberyBackend appends the result as a `tool` role message:

| tool | parameters | execution | result |
|---|---|---|---|
| `call_component` | `spec: ComponentSpec` (docs/components.md protocol) | pushes a render instruction to the frontend via Tauri event; same id = create/update in place | `{ok, rendered/updated/closed: id}` |
| `fetch_terminal` | `{instance, vd_switch}` (vd_switch required, docs/hook.md §VD switching capability) | reads Terminal Content via the Terminal Adapter (docs/terminal-adapter.md); falls back to the latest Context record when unreadable | `{instance, content}` |
| `set_autonomy` | `{key?, motion?, ttlMs?, once?}` | pushes an expression override via Tauri event (semantics: docs/autonomy.md) | `{ok}` |
| `edit_config` | `{action, ...}` (`grep` / `query` / `update`) | discovery, reads, and updates within the restricted Config projection; full schema in docs/toolset.md | the result corresponding to the action |
| `read_memory` | `{...}` | reads Harness-managed persistent understanding Markdown; `index.md` / Memory `AGENTS.md` are read-only by default | `{ok, ...}` |
| `write_memory` | `{...}` | creates or fully replaces one fragmented memory; must carry a description, subject to the per-file length cap | `{ok, ...}` |
| `cron_create` | `{...}` | creates a Harness persistent plan or delayed schedule | `{ok, ...}` |
| `cron_delete` | `{...}` | deletes one persistent plan | `{ok, ...}` |
| `sleep` | `{...}` | waits via the same Harness scheduler, then continues the planned tool sequence | `{ok}` |

Permission boundary: the Tool Set is the entire capability set; there is no tool that modifies code files (the ❌ item in concepts §10a does not exist in the definition table).

## Full turn for one Queue input (the executor of docs/harness.md §trigger model)

A turn is driven by Queue releasing one input, executed serially — while the current turn is unfinished, the next input is not released (concepts §10c, no parallelism):

1. Queue releases one input (with merge Event Buffer → merged into one, if present) → Context writes the input
2. Assemble the system prompt request header on the fly (base_prompt + AGENTS.md + system kaomoji pool, not written to Context; user kaomoji pool queried on demand via `edit_config`; concepts §12)
3. Compression check (auto-compact: Context over threshold → dedicated summary + shaking + reset and re-diff)
4. LLM (request = request header + all Context messages) → with tool_calls: append assistant(tool_calls) + execute in declared order + append the corresponding tool results → call again; without tool_calls: append the assistant message only if content is non-empty, then end. Tool-call budget below.
5. Side effects (Effects) are broadcast to the frontend via Tauri events; the turn ends and Queue releases the next input

### Tool-call budget

Two local Config runtime budgets:

| field | default | effective | agent access | meaning |
|---|---:|---|---|---|
| `max_tool_calls_in_one_response` | 10 | cold | `no_llm_visible` | maximum tool calls executed in one LLM response (≥ 1) |
| `max_tool_calls_per_turn` | 50 | cold | `no_llm_visible` | maximum cumulative tool calls executed while processing one released input (≥ 1) |

Tool calls are still executed serially in the declared order in the response. Calls exceeding either budget are not executed, but each call still writes its corresponding failed tool result; all proposed calls (including unexecuted ones) count toward this turn's budget. When a single response exceeds the budget, calls within budget execute normally, and subsequent calls return a budget error.

When this turn's budget is exhausted, the tool results of executed and unexecuted calls are written to Context as usual; the backend then makes one normal LLM request with empty tools so that it generates a final text reply based on those results. This closing request appends no special system record and cannot initiate another tool call; after the reply, the turn ends normally.

**Silence semantics** (design decision): the LLM returning empty content and no tool_calls = it decided to be silent — Context appends no assistant message ("pet can wake, read, feel no need to disturb, and be silent" concepts §9b).

## Mock Hook Contract (HTTP)

> **The real contract is docs/hook.md** (event layering / session_id identity / marker positioning / startup scan).
> The mock contract in this section is retained as a **debug facility** (manually driving the chain without installing a hook).

```
POST /hook
{
  "event": "session_start" | "stop",
  "instance": "demo-webapp",        // Code CLI 实例名（tab 名）
  "project": "ambery",       // 项目名
  "content": "……",                      // stop 时：模拟读到的 Terminal Content（真实由 sidecar 读）
  "last_assistant_message": "……"        // stop 时：模拟 hook payload 自带字段
}
```

Handling:
- `session_start` → instance registration (status=idle) + Event Buffer silently books "new instance {instance} registered" — no trigger, pet does not wake; it enters Context the next time Queue releases
- `stop` → instance update (status=idle) + Queue system input "{instance} finished ({len} chars). Assess whether to notify." → written to Context after release → trigger

## Read-channel MapAdapter (case-runner story surface)

A read-channel simulation symmetric to the mock hook: the case-runner's `terminal` step writes to the MapAdapter shared map (`docs/terminal-adapter.md` §implementation); both the Timer fallback scan and `fetch_terminal` read it. Production/default builds do not include this injection surface.

## Tauri IPC protocol (frontend–backend communication)

The frontend and core communicate via Tauri native IPC; only external hook scripts use HTTP.

```
Tauri command（前端 → Rust）：
  invoke("get_state")          → TopState（instances + pendingNotifications）
  invoke("get_context")        → ContextMessage[]（对话历史投影）
  invoke("append_user", {text}) → 用户输入入 Queue 排队 → 放行后写 Context user role + 触发
  invoke("push_event", {action, card_id?, ...}) → 结构化用户动作（写 Event Buffer，docs/effect-reporting.md）
  invoke("get_config")          → AppConfig

Tauri event（Rust → 前端）：单事件 `listen("effect", {kind, ...})`，按 kind 判别：
  render_component {spec} / close_component {id} / set_autonomy {face?, motion?, ttlMs?}
  top_state {state} / context_changed {} → 前端收到后重新 invoke("get_context")
  config {} → 裸信号，按需重拉 invoke("get_config")

HTTP（127.0.0.1:47600，仅外部 hook 脚本使用）：
  POST /hook         → hook 脚本触发（fire-and-forget）
```
