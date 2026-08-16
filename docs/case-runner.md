# Case Runner

English | [中文](case-runner.zh.md)

Storage snapshot-driven regression testing and concept observation tool; also hosts the CLI decision source (docs/debug-agent.md) and the full router (docs/core-server.md).

## Principles

> **Scope of this document** — this document defines only the runner infrastructure: run mechanism, sandbox isolation, `.case` format, step, observe, health, export, and CLI; it does not define the capabilities an agent should have, capability planning, or Case coverage strategy — see `docs/capability-evaluation-project.md`.

> **Snapshot is truth** — the case's data section preserves JSONL raw text; line order/timestamps/fields are not lost; details in §Snapshot is truth.

> **Boundary isolation** — one-shot sandbox (production is never written) + headless (never starts a real OS UI); details in §Boundary isolation.

> **Frontend headless observation** — observation boundary and integration shape (headless JS + RemoteBridge connecting to the case-runner's embedded core + mock window layer, i.e. the `app/test/` vitest suite); details in §Frontend headless observation.

> **Shell analogy** — the case-runner is analogous to the Tauri shell: the process body embeds core (the same run_core), spawns a TS test process on demand, and TS connects to the embedded core via RemoteBridge — the same shape as "shell embeds core + shell drives WebView". Corresponding to Tauri multi-window (each window an independent Renderer), one TS test process simulates one window's JS runtime; multi-window scenarios use multiple TS test processes (multiple nodes), each connecting to the shared embedded core via RemoteBridge. TS is not a resident environment; it is one link in the case-runner flow.

> **LLM modes** — two equal modes (debug / real), declared explicitly in the case header; no implicit default; details in §LLM modes.

> **5 core concepts** — case / observe / chat(=user+real llm) / toolcall / worker are first-class objects.

> **Observability system** — all modules are observable, observe has two kinds of output, effects go into observe; details in §Observability system.

> **Tauri runtime actions observable** — non-readonly Tauri runtime actions uniformly enter the effect stream; details in §Tauri runtime actions observable.

> **case privacy boundary** — a case never carries sensitive information (apikey / project names, etc.); details in §case privacy.

## Philosophy

case = storage snapshot + meta, with no embedded assertions. The runner does not judge right or wrong — it observes the behavior of concept structures:
whether Context can replay, whom panorama contains, what timer scan produced, what the queue/event_buffer states are.

**Boundary isolation**: the runner only reads/writes a one-shot sandbox (`%TEMP%\ambery-case-<case_id>/`, rebuilt at start): storage snapshot, runtime appends, and config writes are all inside the sandbox; production storage/config is **never written**. **Headless**: never starts a real OS UI (windows/displays/input); the frontend/OS layer is never within the case observation scope. The only exception: real LLM mode only reads the llm section (providers) of the production config + sends network requests via env keys. debug mode has zero network and is fully deterministic.

## Frontend headless observation

The case-runner's observation scope covers frontend TS capabilities: a headless JS runtime runs real frontend modules (vitest + jsdom) + RemoteBridge connects to the embedded core + mock window layer (§Run shape).

### Observation boundary

| effect | observation method |
|---|---|
| `render_component` / `close_component` (backend) | sandbox effect.jsonl |
| `window_opened` / `window_closed` / `window_visible` / `window_hidden` / `window_moved` / `window_resized` (frontend window layer) | implemented: the headless/browser window layer produces window_* effects via `tauri_runtime_actions`, entering the action stream via RemoteBridge `POST /effect` (`app/test/window-case.test.ts`) |
| `window` (observe target) | implemented: prints the window action sequence and asserts invariants — a `render_component` for a closed window must be accompanied by a new `window_opened`; violation = FAIL |
| frontend non-window logic (store / window wiring) | `frontend-case.test.ts` (headless JS + mock window layer) |

### Frontend read architecture (store convergence + invoke rules)

**store mechanism** (`app/src/store.ts`):
- one frontend store holds readable state owned by core: `config` / `top_state` / `context` / `cards`;
- the store is refreshed by bridge read methods (baseline pulled once + event-triggered on-demand re-pull/direct write); components only take the baseline via `store.<getter>` and subscribe to changes via `store.onX(cb)`, never calling Tauri APIs directly;
- store boundary criterion: **owned by core + read by multiple windows/components + changes drive UI + size manageable**; frontend-local transient state (panel toggles/input boxes/dragging positions) and intents (invoke write commands) do not enter the store. Counterexample: the settings panel schema (single menu consumer, large) does not enter the store; it is read directly via bridge `getConfigSchema()`;
- `BrowserMockBridge` is a store in the "hold + notify" shape; each window has its own store instance, fed through its own bridge.

**invoke rules**:
- reads → always go through the store (reads outside the store go through bridge read methods); writes → always go through the action layer (`tauri_runtime_actions.ts`) or bridge write methods;
- main code logic must not contain `invoke("get_*")` / bare `invoke` scattered calls;
- invoke is allowed only at two choke points: **store refresh** (bridge read method implementation) + **action-layer write commands**.

### Window decisions lifted up

Window existence is decided by the `ensure_card_window` / `close_card_window` commands; the Rust-side authoritative registry (`CardWindowRegistry`) synchronously decides create / reuse / close and returns `opened` / `reused` / `closed` / `absent`.

1. **Close path consolidated**: `close_card_window` consolidates the three paths agent close / shelf dismiss / user ×; destroy does not go through `onCloseRequested`, eliminating preventDefault zombies.
2. **Removal goes through the event loop**: `destroy()` takes effect asynchronously after dispatcher dispatch — the instantaneous view still has a "dying window" window period.
3. **`Closing` state absorbs the window period**: close marks `Closing` → destroy → wait for physical removal (fallback timeout) → leave the registry.
4. **ensure semantics**: if `Closing`, wait for it to leave the registry before recreating; if `Alive` but the registry has no window, self-heal by recreating.

### Run shape (headless JS + RemoteBridge)

The `ambery-case frontend` subcommand embeds core and spawns node + vitest + jsdom subprocesses (sandbox storage/config; bind 0 picks a free port to avoid production 47600); real frontend modules connect to the embedded core via RemoteBridge; `shim.ts` provides the mock window layer; `frontend-case.test.ts` is the first suite, covering store baseline and read-back, Queue release read-back, same-id no-duplicate/no-revive (DOM layer), unified close, and windowed not subscribing to the global stream.

The shape integrated into observation (§Shell analogy):

```
headless 前端 case =
  case-runner（`ambery-case frontend`：内嵌 core，拉起 vitest 子进程）
  + 真实前端模块（vitest + jsdom）：store / RemoteBridge / pet 浏览器分支 / ChatPanel / ComponentManager
  + RemoteBridge（createBridge 无 __TAURI_INTERNALS__ 时自动选中，HTTP+WS 连内嵌 core）
  + shim.ts：端口接线 + core 就绪等待 + 沙盒 effect.jsonl 读取 + MockWindow 注册表（不拦截 Tauri API）
  断言面：DOM 状态 + store 投影 + 沙盒 effect.jsonl（/debug/effect 注入驱动，不落 jsonl）
```

A webview must be attached to a real window; Tauri/wry has no headless webview (see wry discussion #373).

## Layout

Workspace root `Cargo.toml` (members: core / observe-derive / ambery-case; exclude app/src-tauri shell built independently):

```
ambery-case/                    ← workspace member
├── Cargo.toml                    ← deps: ambery-core (features=["case-runner"])
├── cases/                        ← .case 文件（两段式），一个 case 一个文件（gitignore）
│   └── closed-stale-cache.case
├── src/
│   ├── main.rs                   ← CLI 入口（运行/health/export 参数、沙盒 setup、step 循环）
│   ├── runner.rs                 ← step 执行器（load / timer_scan / hook / trigger / user / tool_call / store / terminal / terminal_gone / observe 打印）
│   └── export.rs                 ← 实时 storage → case 导出管线
└── README.md
```

```
ambery-core (feature "case-runner"):
  src/
    case.rs                       ← 两段式解析 + CaseStep/meta + CaseObserve 组装 + pre_parse_check
    eval.rs                       ← 求值引擎（Parser trait/四 parser/变量/类型/DirectToString，case-eval-system.md）
    observe.rs                    ← Observable trait + 各模块投影（observability.md）
    lib.rs                        ← cfg(feature = "case-runner") 暴露；Harness derive(Observe) 覆盖断言
observe-derive/                   ← proc-macro（derive(Observe)，case-runner feature 可选依赖）
```

> The step executor is on the ambery-case (binary) side, not in core: sandbox/terminal story state/printing are CLI concerns;
> core only carries the concepts (parsing/evaluation/observation), keeping the library pure.

## Case file format (two-section, `.case` suffix)

**Snapshot is truth**: the case's data section preserves JSONL raw format (not parsed into objects), line order as-is, timestamps exact, no fields lost. A moment-slice of real storage is imported → manually minimized → sealed as a case; read-channel state (which instance has content on screen, which tab has died) is not expressed by the data but **expressed by the steps story**.

**JSON header + pure JSONL data area**. The header is normal pretty JSON (meta / config / steps),
and the data area is sectioned by `{"__section":"=== name ==="}` marker lines; each section is JSONL **raw text** —
one line per record, zero escaping, scannable and point-editable. The file as a whole is **not valid JSON** (hence the `.case` suffix,
so tooling will not misparse it as JSON).

> **Inspiration**: glTF's `.glb` (Binary glTF) — a JSON chunk describes structure, and a BIN chunk carries raw
> payload, without JSON escaping the binary inline. This format is isomorphic: the JSON header describes the scenario (meta/steps), and JSONL
> sections carry the snapshot raw, instead of escaping JSONL into one giant JSON string.

Parsing rule (the only one): **before the first `{"__section":` line (no indentation, at line start) = JSON header;
after it, sections are divided by marker lines, and data lines belong to the current section**. The marker line itself is valid JSONL (zero foreign syntax).

```json
{
  "meta": {
    "case_id": "closed-stale-cache",
    "created": "2026-07-28T14:30:00Z",
    "llm_mode": "debug",
    "notes": "UIA实证已不存在，storage 仍标记 processing。重现：timer scan 时 terminal_reader 返回旧缓存内容 → 不触发 closed"
  },
  "config": { "timer.interval_ms": 5000, "timer.tick_ms": 5000 },
  "steps": [
    { "load": {} },
    { "terminal": { "instance": "timer-probe", "content": "旧缓存内容——UIA 已不存在但 read_tab 仍返回" } },
    { "observe": ["agents", "panorama"] },
    { "timer_scan": {} },
    { "observe": ["agents", "panorama"] }
  ]
}
{"__section":"=== work_agents ==="}
{"hash":"d5ca2b62","name":"timer-probe","project":"filter-test","status":"processing","last_seen":1785123664753}
{"hash":"1a2887ef","name":"timer-probe","project":"filter-test","status":"processing","last_seen":1785123708559}
{"__section":"=== context ==="}
{"type":"message","role":"system","content":"实例全景同步...","ts":1785163138795}
{"__section":"=== queue ==="}
```

#### ⟡ Consistency analysis

The case's JSON header only describes the scenario and steps; the Storage snapshot must enter the container as raw payload in its real format, rather than escaping Markdown and other non-JSONL files into JSON strings. JSONL and Markdown belong to their own raw areas and boundary rules, so the format can simultaneously keep manual minimization, readable diffs, and file-level replay. If a case is to verify a persistent concept, it should be able to carry its minimal snapshot; export uses inclusion bool plus category filters to control scope, so that "observable" does not become carrying all private data by default.

### Header (JSON)

| field | required | description |
|------|------|------|
| `meta.case_id` | ✓ | unique identifier; kebab-case recommended |
| `meta.created` | ✓ | ISO 8601 |
| `meta.notes` | | scenario description, expected behavior, remarks |
| `meta.llm_mode` | ✓ | debug / real (no default; a missing declaration is invalid; see §LLM modes) |
| `config` | | full-field override (applied path by path through the unified apply_config_by_path pipeline) |
| `steps` | | linear step sequence (see below) |

#### LLM modes

Two equal modes, no implicit default; the case header meta declares `llm_mode` (debug / real):

- `debug`: DebugAgent, zero network; decisions are injected by an external decision source (silence / script / CLI), keeping determinism
- `real`: OpenAI-compatible real endpoint — "I need a real model" is an intrinsic input of the case, recorded in meta `llm_mode`; actual providers are merged from production providers (a declared provider must be a **subset** of production providers, only choosing ones already configured in production, not introducing new providers); keys come only from environment variables

A missing `llm_mode` declaration → invalid case (error), with no implicit fallback to either mode. metrics-type cases (token impact / answer accuracy) depend on real; under debug, user steps follow the decision source's behavior (silence or script).

**no_case_visible**: sensitive fields such as `llm.providers.*` (including base_url / api_key_env) are **forbidden** in a case — write/override validation rejects them, and a case never carries an apikey.

`tool_call` is a mechanical step that **directly executes** a backend tool: it bypasses the LLM, does not write assistant(tool_calls) / tool result into Context, and does not simulate the protocol of "an earlier response saw the query result". It is therefore suitable for testing a tool's independent execution effect, not for verifying an agent's multi-response tool strategy.

Changing config mid-story needs no dedicated step — when testing a single backend write, use `tool_call` to invoke `edit_config`; it shares the Config pipeline with pet's production modifications. When testing agent read-then-write, snapshot reads, budgets, or the tool result chain, use an explicit real / scripted LLM response so that `run_trigger` goes through the full assistant tool_calls → Context tool result → next response flow, rather than splicing two `tool_call` steps.

### Data sections (Storage snapshot)

JSONL sections preserve raw text; every section may be empty — an empty section keeps its marker, so "deliberately empty" is distinguishable from "forgot to fill". Memory / Cron may also enter the case as initial snapshots, but export does not export them by default; they are written to the case only after passing both an explicit inclusion bool and that category's filter.

| section | corresponding storage file | export boundary |
|------|-------------------|----------|
| `work_agents` | work-agents.jsonl | **filtered by default**; exists only after explicit inclusion + filters such as `--instances` |
| `context` | context.jsonl | regular snapshot filtering |
| `queue` | queue.jsonl (Queue input queuing records) | regular snapshot filtering |
| `memory` | memory/ | **filtered by default**; `--keep-memory` + `--memory <name,...>`; only selected normal memories and explicitly selected `AGENTS` enter the snapshot; `index.md` is rebuilt in the sandbox |
| `cron` | cron.jsonl | **filtered by default**; `--keep-cron` + `--cron-ids <id,...>`; each selected id keeps the complete lifecycle event chain |

The memory section is a Markdown raw area (§Consistency analysis: JSONL and Markdown belong to their own raw areas and boundary rules): each file starts with a `{"__file":"<path>"}` marker line (valid JSONL, the same marker philosophy as `__section`), and the lines until the next marker are that file's raw text — blank lines preserved, zero escaping. Path whitelist: `AGENTS.md` or `notes/<name>.md` (reserved names cannot be note paths); `index.md` does not enter a case (the sandbox rebuilds it from selected normal memories), and `cards/` does not go through this section (contract defined separately). Raw lines must not begin with `{"__` at line start — colliding with section/file marker syntax would silently truncate the file; parse rejects it outright. cards/ does not go through this section (for the Card file contract, see docs/components.md §Card file; a case carries no card snapshots).

### case privacy

A case is a shareable/archivable scenario snapshot; privacy details:

- **LLM mode recorded in meta**: `llm_mode` (debug / real) is in the case header meta; the case carries no provider config / keys
- **no_case_visible**: sensitive fields such as `llm.providers.*` (including base_url / api_key_env) are forbidden in a case (write/override validation rejects them) — a case never carries an apikey
- **work_agents filtered by default**: contains the project name `project·sid8` (exposes project structure); not exported by default; exists only when explicitly declared
- **Memory filtered by default**: long-term understanding may contain cross-project facts and collaboration preferences; requires the double explicit selection `--keep-memory` + `--memory <name,...>`. Normal memories are selected by name; Memory `AGENTS.md` keeps raw text only when `AGENTS` is explicitly written; the derived `index.md` is not exported and is rebuilt in the sandbox from selected normal memories
- **Cron filtered by default**: plan messages may contain work content; requires the double explicit selection `--keep-cron` + `--cron-ids <id,...>`. Selected plans keep the complete create / fire / delete lifecycle events; the causal chain must not be truncated by time windows
- **context / queue kept**: real conversation/input snapshots ("snapshot is truth"), shared with trusted parties
- docs/examples do not show real provider names (privacy)

### steps

steps are a linear sequence; each step has a type + optional parameters. The runner executes them in order.

| step | parameters | purpose |
|------|------|------|
| `load` | — | replay the storage snapshot and rebuild all concept structures: Harness::load → in-memory queue/context/filtered_content/event_buffer/agents |
| `terminal` | `{instance, content}` | read-channel story: set that instance's screen content (timer_scan's terminal_reader returns it) |
| `terminal_gone` | `{instance}` | read-channel story: that instance's tab dies (terminal_reader returns None) — death is an explicit verb, visible at a glance when scanning the script |
| `timer_scan` | — | run one timer cycle: TimerWheel due takes due instances → Some goes to handle_timer_scan (change detection enqueues) → None judges mark_instance_closed; finally drain_queue releases |
| `hook` | `{event, name, project, content, ts?}` | inject a mock hook event: handle_hook (by event layering: session_start silent booking / stop enqueues) → drain_queue releases |
| `trigger` | — | drain_queue: release all pending inputs → LLM/effects |
| `user` | `{text, ts?}` | user message enqueued (enqueue user role) → drain_queue releases |
| `tool_call` | `[name, args_json]` | directly execute a tool call, bypassing the LLM: execute_tool → result + effects; does not write tool protocol messages; cannot verify multi-response tool interaction |
| `store` | `{"<name>": {"type":"expr\|var\|int\|str", "value":"<字符串>"}}` | set user variables: value is evaluated by the corresponding parser → to_string → stored as string (see case-eval-system.md) |
| `observe` | `[{"target":"agents"}, {"target":"context","lines":"[$tail-50,$tail]"}]` | record the current snapshot of concept structures (unified object list; path-type targets may carry lines reads, see case-eval-system.md) |

Read-channel default state: any instance not configured returns `None` (= tab no longer exists). Zombie-type cases need no
`terminal` step — by default all are dead, and timer_scan goes straight to the closed judgment.

### observe output

**Observability system**: all concept modules are observable (for the coverage mechanism see `docs/observability.md`); observe outputs two kinds (values: agents / queue / event_buffer / usage / answer / panorama / memory / cron / cards + computed-on-demand filtered_content; paths: context / effects give file pointer + summary); effects go into CaseObserve (exhaustive match enforced at compile time) + full persistence in effect.jsonl (including assistant_delta/done, docs/storage.md §effect.jsonl); filtered_content is not persisted, computed on demand from the terminal-content.jsonl raw digest.

**Tauri runtime actions observable** — `(Tauri runtime actions − readonly) ⊆ effects`: Tauri runtime actions cover WebView's `@tauri-apps/api` and the Rust shell's `tauri` API; all non-readonly actions on both sides uniformly enter the effect stream, and readonly calls are observed directly via observe.

| category | example | destination |
|---|---|---|
| non-readonly (has side effects) | WebView invoke writes (append_user / push_event / set_config), WebviewWindow create/close, setSize / setPosition / emit; Rust shell equivalent window show/hide/termination and emit | → effect stream (recorded to core at the call site; one runtime action one effect; high-frequency same-kind actions are packed by rule) |
| readonly (no side effects) | WebView invoke queries (get_state / get_context / get_config / get_config_schema), reading displays / outerPosition / getByLabel, listen; Rust shell queries | → observe observation (not into effect) |

**Subset notation**: non-readonly Tauri runtime actions ⊂ effects; when one call produces multiple runtime actions, each is recorded as a corresponding effect, and readonly goes through observe value/path observation.

Each observe step prints per requested item in sections (content-level, full text untruncated):

| observe item | category | output content |
|------------|------|----------|
| `agents` | value | all agent entries, including hash / name / project / status / last_seen |
| `panorama` | value | `panorama()` projection text (non-Closed instance summary; with no live instances shows `(无存活实例)`) |
| `context` | path | without lines: `文件路径 \| N 行 \| M tokens（真值 P + est 增量 D）` (truth anchor and est delta annotated separately); with lines: context.jsonl slice raw text (with line numbers) |
| `filtered_content` | value (computed) | full Filtered content archive (instance / source / normalized full text / ts) — the terminal content the agent actually reads |
| `queue` | value | Queue's currently pending inputs (role / content) |
| `event_buffer` | value | Event Buffer's current backlog raw text |
| `usage` | value | truth value of the most recent LLM call (prompt_tokens / completion_tokens / ts; no truth shows `(无)`) — the direct gauge for "scan/answer impact on tokens" |
| `memory` | value | Memory index summary (entry name / description / total count); does not expand Markdown bodies by default |
| `cron` | value | current persistent plan projection (id / schedule / message / next_due); excludes in-process, non-persistent sleep waiters |
| `cards` | value | Card registry projection (id / type / title / created / user_closed / layout summary); does not expand component full text (raw text in sandbox memory/cards/) |
| `effects` | path | without lines: `文件路径 \| N 条`; with lines: effect.jsonl slice raw text (with line numbers, origin / kind / payload / ts) |
| `answer` | value | last assistant message raw text (none shows `(无)`) — the "answer accuracy" scan position |

Value-type targets with lines are invalid (statically intercepted by health pre-parse). CLI output format:

```
── observe @ step 4 ──
agents:
  timer-probe (d5ca2b62) [processing] last_seen=1785123664753

panorama:
  实例全景同步（归零重 diff，1 个存活实例）：
  - timer-probe [Processing] project=filter-test

context: %TEMP%/ambery-case-x/context.jsonl | 12 行 | 3100 tokens（真值 3000 + est 增量 100）

effects: %TEMP%/ambery-case-x/effect.jsonl | 4 条

context: …/context.jsonl | 切片 [74,122]（共 122 行）   ← 带 lines "($cursor,$tail]" 时
  74: {"type":"message","role":"user","content":"…","ts":…}
```

## Export tool: live storage → case

Construct cases from real storage files; reduce volume through fine-grained filtering and minimize manual repair.

### Pipeline

```
实时 storage → 过滤 → 最小化 → JSON 输出 → 手修 meta/notes → case health → 就绪
```

### Filter parameters

The filtering stage only controls which lines enter the case and does not modify line content. `work_agents`, `context`, and
`queue` filter in a cross-file linked manner — centered on `--instances`, affecting all files that contain instance references.

| parameter | purpose |
|------|------|
| `--instances a,b` | keep only lines for the specified instances. work-agents: match by name; context/queue: filter by the instance markers contained in content |
| `--window 30m` | keep only lines within the N-minute window relative to the latest line |
| `--before 1785164703801` | keep only lines before this ts |
| `--after 1785164651768` | keep only lines after this ts |
| `--keep-last N` | work-agents per hash / context content lines per instance keep only the last N lines |
| `--keep-agents` | by default the work_agents section is not exported (contains project name, privacy); only when explicitly specified export the full work_agents section |
| `--keep-memory` | open Memory export eligibility; must be paired with `--memory <name,...>`, otherwise error; no Memory file is brought in by default |
| `--memory name-a,AGENTS` | Memory file filter (only with `--keep-memory`): normal memories selected by name; the reserved value `AGENTS` can explicitly bring in the user-maintained Memory navigation raw text; `index.md` is not selectable and is not exported, rebuilt in the sandbox from selected normal memories |
| `--keep-cron` | open Cron export eligibility; must be paired with `--cron-ids <id,...>`, otherwise error; no plan is brought in by default |
| `--cron-ids id-a,id-b` | Cron plan filter (only with `--keep-cron`): select by plan id; the selected id's create / fire / delete lines are kept complete, not truncated line by line by time windows |
| `--trim-context` | deep trim after cross-file filtering (requires --instances): content lines keep only retained instances (orphan line cleanup); message/queue lines filtered by instance-name mentions; autonomy/session/head/compact_boundary assembly traces dropped (case replay does not load them into memory; do not use for multi-run slice cases) |

### Minimization parameters

The minimization stage slims line content after filtering (does not change structure) to reduce manual repair:

| parameter | purpose |
|------|------|
| `--dedup` | keep only the earliest one of adjacent content-identical lines |
| `--strip-content N` | truncate each context line's content to N characters (full ts, type, role, and other metadata preserved) |
| `--dry-run` | preview filtered per-file line counts without generating a case file |

### Manual repair

In the exported JSON, the `data` fields are all hand-editable — delete redundant lines, add notes, and construct read-channel story with `terminal` / `terminal_gone`
steps. After manual repair → immediately run case health validation.

### Example

```bash
# 从当前 storage 导出场景（--keep-agents 显式保留 work_agents；默认隐私过滤）
ambery-case export --case-id closed-stale-cache \
  --instances timer-probe,full-body-check \
  --keep-last 1 --keep-agents --trim-context

# 预览行数
ambery-case export --case-id closed-stale-cache --instances timer-probe --dry-run
# → work_agents: 2 行, context: 45 行（已 trim）, queue: 8 行（dry-run 预览，未生成 case）

# 验证 case 合法性
ambery-case cases/closed-stale-cache.case --health
# → PASS
```

## case health

Mandatory after manual repair; verifies whether the case is valid under the current code version.

### health checks

1. **Two-section parseable**: the JSON header is valid JSON; marker lines conform to `{"__section":"=== name ==="}` (no indentation, line start).
   The data area is divided into two kinds of sections: JSONL sections (work_agents / context / queue / cron) where every line (including marker lines) can be parsed by
   `serde_json::from_str`; the memory section is a Markdown raw area, divided into files by `{"__file":...}` markers
   (path whitelist and marker collision enforced by parse), not subject to per-line JSON validation
2. **Harness::load does not panic**: all storage lines replay successfully, no schema errors
3. **Concept structures complete**:
   - Agent: each entry has hash / name / project / status
   - Context: each line has type / ts / content (message type must have role)
   - Queue: each line has role / ts
4. **observe executable**: `observe()` returns snapshots of all concept structures without error
5. **pre-parse preflight** (case-eval-system.md §checkhealth, static, not executed): all expressions (observe lines / store values) try_parse syntax-valid; variable references valid (`$tail` predefined, user variables stored before use); store types valid (expr/var/int/str); types storable (Output implements DirectToString); observe targets valid (observable modules)

### Exit codes

```
0 = PASS（所有检查通过）
1 = FAIL（检查失败，打印具体失败行）
2 = USAGE（参数错误）
```

## CLI

```
ambery-case <case>                    # 执行所有 steps，observe 输出到 stdout
ambery-case <case> --step-num 2       # 仅执行到第 N 步（含 observe）
ambery-case <case> --health           # 验证 case 合法性
ambery-case serve [--brain-addr <url> | --silent]    # 完整 router 宿主（浏览器调试 RemoteBridge；docs/core-server.md）
ambery-case frontend [--brain-addr <url> | --silent] # 前端 headless 模式：内嵌 core + 拉起 vitest 子进程（§壳类比）
ambery-case export [--storage DIR] [--instances a,b] [--window 30m] \
              [--before TS] [--after TS] [--keep-last N] [--keep-agents] [--trim-context] \
              [--keep-memory --memory name-a,AGENTS] [--keep-cron --cron-ids id-a,id-b] \
              [--dedup] [--dry-run] [--case-id ID]   # 从实时 storage 导出 case（stdout）
```

`serve` / `frontend` share the `core::host` assembly skeleton (Config/Harness/LLM/Terminal Adapter → AppState → full router); default port 47600, `AMBERY_PORT` overrides (frontend defaults to bind 0 to pick a free port and avoid production). With llm `active=debug`, both must be given either `--brain-addr` or `--silent` explicitly (same as the debug decision-source rule in §LLM modes).

## Use case: debug_brain.py as LLM (debug mode)

debug_brain.py is a local OpenAI-compatible HTTP server (docs/debug-agent.md); to use it as the LLM, start it manually first, then point the case-runner at it in debug mode:

```bash
# 终端 1：起 brain（打印监听端口；--port 可选，默认 47777）
python scripts/debug_brain.py --port 47777

# 终端 2：case-runner debug 模式，--brain-addr 连它
ambery-case <case> --brain-addr http://127.0.0.1:47777
```

- debug mode must explicitly provide `--brain-addr <url>` or `--silent`, otherwise error (§LLM modes in this document).
- brain has a built-in minimal-threshold decision source: hook content ≥ 80 chars → reply with the notify tool; otherwise silence.

## Build

```bash
# workspace 根（推荐）
cargo run -p ambery-case -- ambery-case/cases/closed-stale-cache.case          # 执行所有 steps
cargo run -p ambery-case -- ambery-case/cases/closed-stale-cache.case --health # case 合法性校验

# crate 内亦可（workspace 感知）
cd ambery-case
cargo build
cargo run -- cases/closed-stale-cache.case
```

`ambery-core` must be compiled with the `case-runner` feature: `cargo build --features case-runner` (`ambery-case` already enables it automatically).
