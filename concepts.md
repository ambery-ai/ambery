# Concepts

English | [中文](concepts.zh.md)

## Concept List

### 1. AmberyBackend — system
The backend process. Manages the lifecycle of monitored sessions: receives external inputs, reads source content, and stores it into Context. Executes tool calls issued by pet (LLM), manages persistence, and runs the trigger loop. Loads Config at startup and listens on the HTTP hook port at runtime.

### 2. pet — ui
Ambery's human-machine interface, with a built-in LLM, existing in its own floating window (pill-shaped, always-on-top, draggable; the window shows only kaomoji and no other UI elements; scaling is controlled by Config's view_scale). It expresses state through kaomoji and presents information through Components. The user can type-chat with it — pure natural language, no commands. pet understands user intent, analyzes Context, and decides how to express itself. It does not modify code files; its permission boundary is defined by Harness's Tool Set.

### 3. Surface — ui
A user-visible, hideable, restorable logical interface: Chat and Card are Managed Surfaces (unified show / hide / restore semantics); pet itself is the anchor and interaction entry, not a Surface; OS Window is only a physical projection of a Surface; Cards Shelf and Menu are transient popups, not Surfaces.

#### 3a. Chat Panel — subconcept
The interface where the user type-chats with pet. Summoned by pet right-click, not a Component. It includes an input box and conversation history (read from Context). User input is written into Queue for queuing and, after release, enters Context as a `user` role message. Assistant replies stream incrementally: SSE fragments are pushed directly to the frontend via StreamingChannel and rendered piece by piece — a pure display optimization — without going through Queue/Context; the full reply is written to Context only at the end.

### 4. Autonomy — system
pet's autonomous behavior engine. Controls facial expression switching and the floating movement of the pet window. Two control paths: default automatic behavior is defined by the kaomoji mapping table in the system prompt (e.g. Processing → `(ˇωˇ」∠)_` + slow floating, has notification → `✧*｡٩(ˊᗜˋ*)و✧*｡` + bouncing), and does not depend on the LLM; pet can also actively override it via the `set_autonomy` tool call (e.g. suddenly jump, change expression to act cute).

Autonomy's top-level state is expression and animation. State uses keys, not the kaomoji text itself — format `[face: idle, motion: still]`, 4 words + 5 symbols, about 6-7 tokens. It is independent of Queue — whether the state changes or not, it is appended directly to the end of the request context each round, persists in Context (one record per round), and does not enter Queue. The volume is tiny, so there is no cache concern.

> **Autonomy is its own engine, independent of AmberyBackend.** Its state does not go through Queue and enters the context directly.
>
> **Autonomy does not read monitored sessions.** State key transitions (Processing → Idle, etc.) are driven by AmberyBackend based on evidence pathways (hook / scans); Autonomy only outputs the expression and motion matching the current key.

### 5. Component — ui
A predefined frontend visual card type used for information display. Invoked by pet through Tool Set, and popped out offset from pet toward a suitable direction. The Component protocol defines two layers: ① the system prompt describes type/parameters/usage scenario/text volume ② a CLI-style function call in Tool Set, where the spec parameter is the context content. User interaction events (close, jump, check, etc.) do not write a `user` role and do not go through Queue; they write to Harness's Event Buffer — attached into Context when Queue releases.

### 10. Harness (context manager) — system
The data and capability layer pet needs to run. Queue is the global serializing entry — AmberyBackend writes external inputs and user messages into Queue. Queue releases serially → Context writes input → LLM → Context writes output → release the next one. Event Buffer is an independent input channel parallel to Queue, receiving Component interaction events. Context updates and Compression are handled internally by Harness, not operated directly by the backend. pet's LLM is based on the OpenAI Chat Completions API model:

- **messages**: the message array, each entry containing `role` (`system` / `user` / `assistant` / `tool`) and `content`
- **tools**: the list of available function definitions, which the LLM invokes by emitting `tool_calls`
- **tool result**: after AmberyBackend executes a tool, it appends a `role: "tool"` message back to messages

#### 10a. Tool Set — subconcept
The function definitions pet can call (CLI-style naming): `call_component` / `fetch_terminal` / `set_autonomy` / `edit_config` / `read_memory` / `write_memory` / `cron_create` / `cron_delete` / `sleep`. pet itself cannot perform any operation — it can only emit `tool_calls`; AmberyBackend executes them and appends the result back to Context as a `tool` role message. Tool Set is also pet's permission boundary: ❌ modifying code files.

#### 10b. Context — subconcept
pet's external information injection channel, persisted in Harness. Each record carries a timestamp. Context is the complete message array (mirroring OpenAI messages), the context source for LLM requests, and the persistent archive of the full conversation. The data model and processing chain belong to the design docs. Through Context, pet obtains non-conversation information such as monitored-session states and task background.

#### 10c. Queue (input serialization gate) — subconcept
The serial Queue for all inputs, persisted in Harness. External inputs, user messages, and merged Event Buffer events all enter Queue first. **Queue is the controller of the system's processing rhythm** — after each input is released, the entire round must finish (input written to Context → LLM call → tool execution → assistant output written to Context) before the next one is released; the OpenAI API must not be called in parallel. The LLM's assistant replies and tool results do not go through Queue and enter Context directly.

```
Input1 → Queue → Context → LLM → tool_calls? → call LLM again ─→ output → Context ✓
Input2 → Queue (waiting in line)─────────────────────────────────────→ released → Context → LLM → ...
```

#### 10d. Compression — subconcept
Triggers when Context exceeds the token threshold (auto-compact): measured by the usage truth, threshold calibrated by the model's `context_window`, a dedicated LLM call generates a summary + shaking keeps complete turns + reset and re-diff. It ensures the context of every LLM call stays within budget.

#### 10e. Event Buffer — subconcept
The independent input channel for Component interaction events, parallel to Queue: interactions are written in short natural language (optionally with a structured snapshot attached); when the LLM is triggered, they are merged into one `system` message and attached into Context, then cleared; they never write a `user` role.

#### 10f. Memory (persistent understanding buffer) — subconcept
The persistent buffer the Agent uses to continuously record and recover its own understanding, replacing the Agent directly manipulating the filesystem to write notes. Memory is independent from Context, Compression summaries, and source content archives respectively: the first is the Agent's actively maintained long-term understanding, while the latter are run records or reference data. It is persisted and managed by Harness, manageable by the backend and the user, and adjusted by the Agent through the read/write Memory tools.

#### 10g. Cron (persistent scheduling and delayed dispatch) — subconcept
Harness's persistent scheduling and delayed-dispatch capability. It records future work, for example prompting the Agent with a daily report every night; it also supports continuing a planned action after a short wait, for example waiting a few seconds before calling `set_autonomy`. The backend, the user, and the Agent can all view or adjust Cron; the Agent manages it through the two Cron tools, and the `sleep` tool expresses a short wait. Cron and sleep share the same Harness scheduling implementation.

```
Component interaction ─→ Event Buffer (backlog)
                     ↓ attached when Queue releases
                   Context (merged into a system message)
                     ↓
                   LLM ─→ Context
```

| | Queue | Event Buffer |
|---|---|---|
| Input source | external inputs, user messages | Component interaction events |
| Processing | Serial Queue, processed one at a time | Backlog, merged into one when the LLM triggers |
| Persistence | append-only queue.jsonl | Not persisted (cleared after merge) |
| Output direction | Context archive + LLM request body | Injected into the LLM request context after merge |
| Role | Input serialization gate | Merge staging area for low-priority events |

### 12. Config — system
The persistent configuration for AmberyBackend and pet (LLM profiles, Compression, kaomoji pool, pet appearance, theme, language, pet name, tool budget, etc.). Loaded at runtime. pet can query and modify on demand within a restricted Config projection via `edit_config`; the system-prompt assembly rules belong to the design docs.

### 13. Storage — system
The persistence layer for runtime data. It is the same type as Config (persistent files) but with a different purpose: Config stores startup configuration, while Storage stores session data and Harness persistent state (read/write). Layout and file semantics belong to the design docs. It is preserved across AmberyBackend lifecycles, and full conversations and future plans are restored after a restart.

### 15. Platform Primitives — system
The abstraction group for platform-specific capabilities: the layer that acts in the user's environment on pet's behalf — virtual desktop switching, window focus/activation, and similar OS-level actions. This concept is the reason the shell carries multi-platform handling: Windows / macOS / Web, each with its own implementation (Windows uses COM). Actions that interrupt the user (e.g. switching to another desktop) are gated by explicit consent.

