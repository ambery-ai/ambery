# Agent Loop Design

English | [中文](agent-loop.zh.md)

> - **Scope of this document** — this document fixes the LLM abstraction, the Tool Set protocol, the mock hook contract, the trigger lifecycle, LLM requests, tool-call budgets, and post-exhaustion closing; concept definitions: see [concepts.md](../concepts.md) §pet / §Tool Set; for Config migration, map reconcile, and descriptor mechanisms, see [config.md](config.md).
> - **No code logic that need not exist** — after the budget is exhausted, still use the existing tool result → LLM closing chain: make one final text-reply request normally with empty tools; do not inject an extra terminating system record and do not automatically start a new turn.

## LLM Abstraction

```rust
trait Llm {
    async fn complete(&self, messages: &[ContextMessage], tools: &[ToolDef]) -> LlmOutput;
}
struct LlmOutput { content: Option<String>, tool_calls: Vec<ToolCall> }
```

Implementation and assembly (`LlmBackend::from_config`):
- `active: "debug"` → `DebugAgent`: pure mock, zero logic. Hand the entire Context to an external decision source (silence/script/HTTP brain) and returns verbatim the `LlmOutput` that the decision source gives ([docs/debug-agent.md](debug-agent.md)).
- `active: "<provider name>"` → `OpenAiClient` (OpenAI-compatible endpoint); **on failure, fall back to DebugAgent** (initialization failure: env unset / provider nonexistent → overall fallback; call failure: HTTP/timeout/parse → fallback for that turn). Failures are no longer silent: while falling back, also emit an `llm_error` effect, and pet renders an "LLM call failed" error-frame Card (updated in place under the same id `llm-error`).

**Config v2 (multi-profile + active selector)**: providers stores each provider's `base_url/model/api_key_env/temperature`; switching only changes `active` without losing config; keys live only in the environment (app-level env file or process env — see [docs/llm-setup.md](llm-setup.md) §Key storage model), never in config.json. On first startup, when config.json does not exist, write the default presets (deepseek/moonshot/zhipu/openai/ollama public providers); private providers are added by the user to the local config.json.

**reasoning_content return requirement for thinking models**: when a thinking model has thinking mode enabled, any assistant message with `tool_calls` in the history **must carry the `reasoning_content` field back**, otherwise 400 (empty string passes; `thinking:{type:"disabled"}` is not accepted). Therefore ContextMessage persists `reasoning_content` (old records lack this field; replay pads an empty string), and AmberyBackend stores the chain of thought on every turn's assistant tool_calls message.

**Record ≠ replay**: the reasoning_content of plain-text replies is **also stored to disk at full fidelity** (context.jsonl is auditable; debug/case can inspect the chain of thought), but it is **not replayed** — DeepSeek officially requires not returning reasoning across turns (each turn's reasoning is one-shot), and build_body writes reasoning_content only in the tool_calls branch, so plain-text replies cost no tokens and carry no 400 risk.

## Tool Set Design Principles

> **No special rules; let semantics define behavior** — tool parameters are fully defined by schema; implicit conventions are forbidden (missing parameter switches mode, empty value triggers query, etc.). The single source of implementation is `tool_set()` in `core/src/llm.rs`.

> **Progressive disclosure; query on demand** — Config is deeply nested; the LLM discovers paths and types layer by layer through tool call-feedback, not by relying on injected external Schema.

## Tool Set Protocol (concepts §Tool Set)

Nine function definitions, CLI-style names; after execution AmberyBackend appends the result as a `tool` role message:

| tool | parameters | execution | result |
|---|---|---|---|
| `call_component` | `spec: ComponentSpec` ([docs/components.md](components.md) protocol) | pushes a render instruction to the frontend via Tauri event; same id = create/update in place | `{ok, rendered/updated/closed: id}` |
| `fetch_terminal` | `{instance, vd_switch}` (vd_switch required, [docs/agents/claude/hook.md](agents/claude/hook.md) §VD switching capability) | reads Terminal Content via the Terminal Adapter ([docs/terminal/terminal-adapter.md](terminal/terminal-adapter.md)); falls back to the latest Context record when unreadable | `{instance, content}` |
| `set_autonomy` | `{key?, motion?, ttlMs?, once?}` | pushes an expression override via Tauri event (semantics: [docs/autonomy.md](autonomy.md)) | `{ok}` |
| `edit_config` | `{action, ...}` (`grep` / `query` / `update`) | discovery, reads, and updates within the restricted Config projection; full schema in [docs/toolset.md](toolset.md) | the result corresponding to the action |
| `read_memory` | `{...}` | reads Harness-managed persistent understanding Markdown; `storage/memory/index.md` / `storage/memory/AGENTS.md` are read-only by default | `{ok, ...}` |
| `write_memory` | `{...}` | creates or fully replaces one fragmented memory; must carry a description, subject to the per-file length cap | `{ok, ...}` |
| `cron_create` | `{...}` | creates a Harness persistent plan or delayed schedule | `{ok, ...}` |
| `cron_delete` | `{...}` | deletes one persistent plan | `{ok, ...}` |
| `sleep` | `{...}` | waits via the same Harness scheduler, then continues the planned tool sequence | `{ok}` |

Permission boundary: the Tool Set is the entire capability set; there is no tool that modifies code files (the ❌ item in concepts §Tool Set does not exist in the definition table).

## Full turn for one Queue input (the executor of [docs/harness.md](harness.md) §trigger model)

A turn is driven by Queue releasing one input, executed serially — while the current turn is unfinished, the next input is not released (concepts §Queue, no parallelism):

1. Queue releases one input (with merge Event Buffer → merged into one, if present) → Context writes the input
2. Assemble the system prompt request header on the fly (base_prompt + `config/AGENTS.md` + system kaomoji pool, not written to Context; user kaomoji pool queried on demand via `edit_config`; concepts §Config)
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

**Silence semantics** (design decision): the LLM returning empty content and no tool_calls = it decided to be silent — Context appends no assistant message ("pet can wake, read, feel no need to disturb, and be silent" — concepts, pet).

## Mock Hook Contract (HTTP)

> **The real contract is [docs/agents/claude/hook.md](agents/claude/hook.md)** (event layering / session_id identity / marker positioning / startup scan).
> The mock contract in this section is retained as a **debug facility** (manually driving the chain without installing a hook).

```
POST /hook
{
  "event": "session_start" | "stop",
  "instance": "demo-webapp",        // Code CLI instance name (the tab name)
  "project": "ambery",       // project name
  "content": "……",                      // on stop: the Terminal Content this run pretends to read (a real run reads it via the sidecar)
  "last_assistant_message": "……"        // on stop: a field the simulated hook payload carries
}
```

Handling:
- `session_start` → instance registration (status=idle) + Event Buffer silently books "new instance {instance} registered" — no trigger, pet does not wake; it enters Context the next time Queue releases
- `stop` → instance update (status=idle) + Queue system input "{instance} finished ({len} chars). Assess whether to notify." → written to Context after release → trigger

## Read-channel MapAdapter (case-runner story surface)

A read-channel simulation symmetric to the mock hook: the case-runner's `terminal` step writes to the MapAdapter shared map ([docs/terminal/terminal-adapter.md](terminal/terminal-adapter.md) §implementation); both the Timer fallback scan and `fetch_terminal` read it. Production/default builds do not include this injection surface.

## Tauri IPC protocol (frontend–backend communication)

The frontend and core communicate via Tauri native IPC; only external hook scripts use HTTP.

```
Tauri command (frontend → Rust):
  invoke("get_state")          → TopState (instances + pendingNotifications)
  invoke("get_context")        → ContextMessage[] (projection of the conversation history)
  invoke("append_user", {text}) → user input queues in Queue → once released, written to Context as the user role and the turn triggers
  invoke("push_event", {action, card_id?, ...}) → a structured user action (writes the Event Buffer, docs/effect-reporting.md)
  invoke("get_config")          → AppConfig

Tauri event (Rust → frontend): a single event `listen("effect", {kind, ...})`, dispatched by kind:
  render_component {spec} / close_component {id} / set_autonomy {face?, motion?, ttlMs?}
  top_state {state} / context_changed {} → the frontend re-invokes ("get_context") on receipt
  config {} → a bare signal; re-pull invoke("get_config") on demand

HTTP (127.0.0.1:47600, used only by external hook scripts):
  POST /hook         → triggered by a hook script (fire-and-forget)
```
