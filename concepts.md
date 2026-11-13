# Concepts

English | [中文](concepts.zh.md)

## Concept List

### 1. Pet
Ambery's human-machine interface, with a built-in LLM, living in its own floating window that shows only kaomoji. It expresses state through kaomoji and presents information through Components. The user can type-chat with it — pure natural language, no commands. Pet understands user intent, analyzes Context, and decides how to express itself and when the user is worth disturbing. It does not modify code files; its permission boundary is defined by Harness's Tool Set. Pet is the agent; how it appears to the user is the Pet Surface (see Surface), which is also the anchor: it is the origin every other Surface's layout is measured from.

#### 1a. Autonomy
Pet's autonomous behavior engine: facial expression switching and the floating movement of the Pet window. Two control paths — the default kaomoji mapping defined in the system prompt (no LLM involved), and Pet's active override via a tool call. Its state is expression and animation, keyed by a face and a motion value, appended to each request and persisted in Context.

> **Autonomy is its own engine** — its state bypasses Queue — and **it does not read monitored sessions**: key transitions (Processing → Idle, etc.) are driven by AmberyBackend from external inputs; Autonomy only outputs the expression and motion matching the current key.

### 2. Window
The host's physical container: the shell creates, sizes, places, shows, hides and destroys it, and it carries no persistent truth of its own — the persistent state of whatever it shows lives in that interface's own record (a Card file, the Config). Every Surface is projected into one, and a Surface may outlive its window: a hidden Card has none. Two windows have no Surface behind them — the Menu and the Cards Shelf — because they hold no state of their own.

#### 2a. Surface
A managed interface with its own display choice and spatial layout. Three families live under Surface: Pet (the anchor), the Chat Panel (conversation) and the Card (a rendered unit of information). The anchor is the origin; every other Surface sits at an offset from it.

##### 2a-1. Chat Panel
The interface where the user type-chats with Pet. Summoned by Pet right-click, not a Component. It includes an input box and conversation history (read from Context). User input is written into Queue and, after release, enters Context as a user role message; assistant replies stream to the user as they are generated — a display optimization — with the full reply written to Context at the end.

##### 2a-2. Card
A Card is the Surface-side identity of rendered information: a Managed Surface (unified show / hide / restore semantics) on which a Component's content becomes visible to the user. User interactions on a Card (close, jump, check, etc.) do not write a user role and do not go through Queue; they write to Harness's Event Buffer. The Card's persistence and state identity — its file, its stable id, its lifetime across restarts — lives in Component State; the two are the two faces of one thing.

#### 2b. Menu
The settings entry, opened from the tray: the schema-driven Config editor.

#### 2c. Cards Shelf
The management panorama over all Cards, opened from Pet: it lists, shows, hides, and restores Cards. Its truth source is the persisted Card collection (see Component State), not its own state.

### 3. Component
The structured content the agent can call: predefined frontend card types used for information display. Component is the data plane, not the interface itself; when rendered, its content appears as a Card on the Surface.

#### 3a. Component State
The persistent identity of a rendered Component. One stable id governs the whole lifecycle: the first call creates, later calls with the same id update in place, an explicit call closes. The Card file is the cross-restart truth, colocating the agent-updatable spec with the user's Surface management state — the agent can update content but cannot silently override the user's display choice. As a persistent work artifact the Card is also a memory-side object — referenced by notes, but not an ordinary Memory note and not managed through the note tools.

### 4. Harness
The external data-and-capability layer Pet runs on — external to Pet, not part of its face. It carries the Tool Set protocol (the boundary of what Pet may do) and hosts five domains: Context (the data plane), the Agent Loop (the mechanism plane), Memory (persistent understanding), Timer (scheduling), and Perception (sensing). Pet's LLM speaks the OpenAI Chat Completions API; the data model and processing chains belong to the design docs.

#### 4a. Tool Set
The function definitions Pet can call, named CLI-style (the full table belongs to the design docs). Pet itself cannot perform any operation — it can only emit tool calls; AmberyBackend executes them and appends the result back to Context as a tool role message. The Tool Set is also Pet's permission boundary: ❌ modifying code files. Among the tools, the perception tools are how the agent actively fetches external content on its own initiative — the Tool Set's hand reaching toward the protocol layer.

#### 4b. Context
The data plane of Harness: the complete message array (mirroring OpenAI messages), the context source for LLM requests, and the persistent archive of the full conversation. Every record carries a timestamp. Through Context, Pet obtains non-conversation information such as monitored-session states and task background. Context is data, not mechanism — the machinery that drives and maintains it lives in the Agent Loop and Compression.

##### 4b-1. Compression
The budget mechanism operating on Context: when the usage truth shows the budget exceeded, a dedicated LLM call summarizes the history and shaking keeps only recent complete turns. Digest is its counterpart on the understanding side — understanding is deposited into Memory first, so shaking details away afterwards is safe.

#### 4c. Agent Loop
The mechanism plane of Harness: the engine that turns one released input into a complete round — assemble the request header, call the LLM, execute tool calls in order, broadcast effects, release the next input. The word is borrowed from industry usage (the agentic loop). Its units and gates are Queue, Event Buffer, and Turn.

##### 4c-1. Queue
The serial gate for all inputs, persisted in Harness. External inputs and user messages enter the Queue; Event Buffer entries never queue — they attach at release time (see Event Buffer). User questions are queued with priority over background inputs; every input carries its source. Queue is the controller of the system's processing rhythm: after each input is released, the entire turn must finish before the next one is released — the OpenAI API is never called in parallel. The LLM's assistant replies and tool results do not go through Queue and enter Context directly.

```
Input1 → Queue → Context → LLM → tool_calls? → call LLM again ─→ output → Context ✓
Input2 → Queue (waiting in line)─────────────────────────────────────→ released → Context → LLM → ...
```

##### 4c-2. Event Buffer
The independent input channel for Component interaction events and silent bookkeeping, parallel to Queue: entries pile up in the Buffer itself and attach at release time as system messages — never entering the Queue, never writing a user role.

##### 4c-3. Turn
One complete processing unit defined by one input: it begins when Queue releases one input and ends when everything that input triggered — LLM responses, tool calls and results, the final reply — has finished. A turn belongs to its input; the tool-call budgets count per turn, Compression shakes only between complete turns, and sleep continues a planned action within the same turn.

#### 4d. Memory
The persistent understanding domain managed by Harness: notes/ hold the agent's long-term understanding, and the workspace persists across turns, compression, and restarts. It has two entries: the agent's active writes through the two note tools and the forced deposits of Digest. Memory is independent from Context, Compression summaries, and source content archives — it is understanding, not run records or reference data. The backend and the user can also manage it.

#### 4e. Timer
Harness's scheduling engine — one engine, three uses: scheduled plans (created and deleted via the cron tools), short waits (continuing a planned action after the wait), and catch-up scans (the default entry shape of a Watch Schedule). The scheduled-plan tools and the wait tool are its handles.

#### 4f. Perception
The Harness-side sense organ facing the Ambery Protocol. It takes delivery of what the protocol layer hands over — a Hook push or a Watch Schedule scan — starts Digest on every incoming source update stream, and renders the digested result into agent-consumable content for the loop. The perception tools in the Tool Set are its hand on the tool side.

### 5. Ambery Protocol
Ambery's outward-facing master contract: the abstraction Ambery makes of external software — what a program satisfies to become a Source, how its host pushes events, how its updates are digested, and how proactive observation is scheduled. This is the package's primary concern; the internal construction of any particular software is out of scope. Transport carriers are deliberately not fixed at the concept layer.

#### 5a. Source
An external entity under observation, as projected into the protocol. A Source occupies a seat in Context via one stable source id: its updates re-enter under the same ID instead of re-introducing themselves — continuity is carried by the ID, and attention is driven by the update stream. A Source has a process basis and a semantic seat, and the two are related but distinct: a host may die while the seat waits for reconnection, and a seat may close while the host lives on.

##### 5a-1. Source Host
The process or window that carries Sources — a terminal window, a browser, a library app, a player. Enumeration, location, activation, and liveness live at this layer. Different software hosts Sources differently; each host's concrete access shape belongs to the access contract, not the concept layer.

###### 5a-1a. Hook
The entry through which host software proactively pushes "something happened" to Ambery — the shoulder-tap channel of the protocol. Every real host has one: lifecycle events, bookmark changes, tab events are all things the host knows first and pushes out. Claude Code's five lifecycle events are the first instance; each host's hook shape (configuration, script, event set) belongs to that host's contract.

##### 5a-2. Context Slot
The seat itself: one stable source id in Context, with the update stream as its import and export. The slot is the protocol's modeling unit for proactive notification — the same ID appearing again and again is what lets the model treat an external thing as continuous. Session identities, card ids, page positions are all instances of the same pattern.

##### 5a-3. Digest
The mandatory processing step for a Source's update stream. When an observation act delivers new content — a Hook push or a Watch Schedule scan — Perception starts Digest; its product is twofold: a Memory note (understanding lands before compression shakes details away) and accumulated per-Source understanding that feeds adjustments to the Watch Schedule. Digest is forced, not left to the agent's discretion.

##### 5a-4. Watch Schedule
The plan set the agent authors for its own proactive observation: which Sources to watch, at what granularity, when, and on what trigger conditions. Fallback patrol polling is only the default entry shape — catch up on a Source whose hook has been quiet too long; event-driven, low-frequency, and paused watching are other shapes. The agent adjusts the schedule from Digest's per-Source understanding, the rhythm of hooks, and the user's cues; Harness Timer is its execution engine.

### 6. AmberyBackend
The backend process that carries the runtime: receives external inputs, executes the Tool Set calls issued by Pet (LLM), and manages persistence. It is the process on which Harness's mechanisms and Pet's loop ride — a carrier, not a domain of its own.

### 7. Config
The persistent configuration for AmberyBackend and Pet (LLM profiles, appearance, language, tool budget, etc.). Loaded at runtime. Pet can query and modify on demand within a restricted Config projection; the system-prompt assembly rules belong to the design docs.

### 8. Storage
The persistence layer for runtime data. It is the same type as Config (persistent files) but with a different purpose: Config stores startup configuration, while Storage stores session data and Harness persistent state (read/write). Layout and file semantics belong to the design docs. It is preserved across AmberyBackend lifecycles, and full conversations and future plans are restored after a restart.

### 9. Session
Pet's own lifetime unit: one AmberyBackend startup opens one session, and every record produced during that run belongs to it — the boundary marker lives in Storage. Logs are sacred and never rewritten; a session is therefore also the replay unit — reconstructing a run means slicing between two session markers. Restoring state after a restart reads across the boundary (Memory, cards, instance list), but the new records belong to the new session; the old session's logs stay intact.

### 10. Platform Primitives
Ambery's own platform-capability layer — Tauri-specific, not an external abstraction of the protocol: everything about Ambery's own windows and environment (positioning, sizing, focus and desktop switching of its own Surfaces) must be implemented per platform by the shell. This concept is why the shell does multi-platform work: Windows / macOS / Web each have their own implementations and capability boundaries (Web is limited). Access to external sources — locating and reading other software — is not here; it belongs to the Ambery Protocol side.

---

## Examples

### Example A: a Claude Code session finishes — the terminal play

Claude Code runs in a terminal pane; the pane's window is the **Source Host**, and the session itself is a **Source** holding a **Context Slot** under its stable session id. The session finishes its work; the host's **Hook** (Stop) fires and pushes the event to **AmberyBackend**. **Perception** takes the delivery and starts **Digest** on the session's update stream: the understanding lands as a Memory deposit, and the digested update enters the **Queue**. The **Agent Loop** releases it — one **Turn**: the request is assembled from **Context**, the LLM judges the news worth telling, and emits a tool call through the **Tool Set**; **Pet**'s **Autonomy** flips to the bouncing face, and the reply renders as a **Card** on the **Surface**: a new **Window** opens beside Pet's, and the **Component** it carries has a stable id, so a later update lands in place with its **Component State** (including the user's display choice) preserved. The user follows up in the **Chat Panel**; the interaction events go to the **Event Buffer** and attach at the next release.

### Example B: a book keeps its place — the reading play

The user reads a novel in a library app; the app is the **Source Host**, and the book holds a **Context Slot** keyed by its title — every page turn is an update under the same ID. A **Watch Schedule** entry the agent authored earlier checks the book between chapters; on each delivery **Perception** starts **Digest**, and the digest deposits a short note into **Memory** (plot position, the user's margin thoughts). Days later the user asks Pet in the **Chat Panel**: "Where was I?" — Pet answers from **Memory**, without re-reading a single page.

### Example C: a player with no hook — patrol as a plan entry

A video player is a **Source Host** that offers no **Hook**; the access contract can only poll. The agent's **Watch Schedule** therefore holds a patrol entry — check progress every five minutes — and the **Timer** executes it on its tick. This entry exists precisely because the host's hook is silent; a host that pushes needs no patrol. Each scan is one **Turn**: the progress update lands on the film's **Context Slot**; at the scene worth telling, **Pet** decides to disturb — **Autonomy** bounces, and a **Card** pops with the time offset in its own **Window**.

### Example D: compression and digest divide the labor

A long session: **Context** has grown until the usage truth crosses the budget, and **Compression** fires inside the **Agent Loop** — a dedicated LLM call summarizes the history, shaking keeps complete **Turns** and drops the rest, and the change-detection baseline resets. Nothing that matters is lost: **Digest** deposited understanding into **Memory** along the way. The user asks in the **Chat Panel**: "What did we conclude about that bug yesterday?" — Pet answers from **Memory**, not from shaken context, then the **Queue** releases the next input and the loop rolls on.

### Example E: bootstrap — the state that survives a restart

**AmberyBackend** starts, loads **Config**, and restores from **Storage**: the **Memory** workspace (notes and index), the instance list, and every **Card** file — content and **Component State** colocated, so the display choices the user made (hidden cards, dragged layouts) survive the restart. The shell rebuilds the **Windows** for Pet, the **Chat Panel** and every visible **Card** — **Platform Primitives** work: placement, size and z-order are each platform's own implementation, never the protocol's. The **Cards Shelf** rebuilds its panorama from the card files; a sick file is skipped, one broken card does not take down the rest. The user opens the **Menu** from the tray and finds the appearance and language set before the restart — the editor is generated from the **Config** schema.

### Example F: one Pet session — where the word applies

The user starts Ambery; **AmberyBackend** opens **session** #42: one line in Storage marks the boundary, and every record this run — Context messages, Event Buffer merges, turns — belongs to it. Mid-session the user restarts the app after a crash; a new session #43 opens, and the restored state (Memory workspace, Card files, instance list) carries over — the conversation continues because Context is reloaded from Storage, but the session boundary is drawn: session #42's logs stay intact for replay, and everything after the restart is #43. Session is Pet's own lifetime unit — one per startup — and it is what makes "reconstructing a run" a slicing operation between two session markers.

---

## Concept × Example Coverage

| Concept | A: terminal play | B: reading play | C: player patrol | D: compression × digest | E: bootstrap | F: Pet session |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| Pet | ✅ | ✅ | ✅ | ✅ | — | ✅ |
| Autonomy | ✅ | — | ✅ | — | — | — |
| Window | ✅ | — | ✅ | — | ✅ | — |
| Surface | ✅ | — | ✅ | — | ✅ | ✅ |
| Chat Panel | ✅ | ✅ | — | ✅ | — | — |
| Card | ✅ | — | ✅ | — | ✅ | ✅ |
| Menu | — | — | — | — | ✅ | — |
| Cards Shelf | — | — | — | — | ✅ | — |
| Component | ✅ | — | ✅ | — | — | ✅ |
| Component State | ✅ | — | — | — | ✅ | — |
| Harness | ✅ | — | ✅ | ✅ | ✅ | — |
| Tool Set | ✅ | — | — | — | — | ✅ |
| Context | ✅ | — | — | ✅ | — | — |
| Compression | — | — | — | ✅ | — | — |
| Agent Loop | ✅ | — | ✅ | ✅ | — | — |
| Queue | ✅ | — | ✅ | ✅ | — | — |
| Event Buffer | ✅ | — | — | — | — | — |
| Turn | ✅ | — | ✅ | ✅ | — | — |
| Memory | — | ✅ | — | ✅ | ✅ | — |
| Timer | — | ✅ | ✅ | — | — | — |
| Perception | ✅ | ✅ | ✅ | ✅ | — | — |
| Ambery Protocol | ✅ | ✅ | ✅ | ✅ | — | ✅ |
| Source | ✅ | ✅ | ✅ | — | — | ✅ |
| Source Host | ✅ | ✅ | ✅ | — | — | ✅ |
| Hook | ✅ | — | ✅ | — | — | — |
| Context Slot | ✅ | ✅ | ✅ | — | — | — |
| Digest | ✅ | ✅ | ✅ | ✅ | — | — |
| Watch Schedule | — | ✅ | ✅ | — | — | — |
| AmberyBackend | ✅ | — | ✅ | — | ✅ | — |
| Config | — | — | — | — | ✅ | — |
| Storage | — | — | — | — | ✅ | — |
| Session | — | — | — | — | ✅ | ✅ |
| Platform Primitives | — | — | — | — | ✅ | — |
