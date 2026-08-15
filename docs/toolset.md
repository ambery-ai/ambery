# Tool Set

Nine function definitions callable by pet (call_component / fetch_terminal / set_autonomy / edit_config / read_memory / write_memory / cron_create / cron_delete / sleep). The validation rules define legality and error returns at backend execution time.

## Scope of this document

This document defines each tool's parameter schema, return structure, and call semantics; for tool turn budgets and trigger termination policy see `docs/agent-loop.md`.

## call_component

Creates / updates / closes Card windows. The same id creates on first use and updates in place afterwards. The tool schema declares the complete fields of each type via `anyOf`.

| Parameter | Type | Required | Validation |
|------|------|------|------|
| `spec.id` | string | ✓ | `[A-Za-z0-9_\-/.]+`, not empty. No spaces, Chinese characters, or special characters |
| `spec.type` | string | ✓ | `text_card` / `quick_jump` / `git_display` / `data_chart` / `todobox` |
| `spec.direction` | string | | `auto` / `n` / `ne` / `e` / `se` / `s` / `sw` / `w` / `nw` |
| `spec.title` | string | required when type = text_card/git_display/data_chart/todobox | not empty |
| `spec.text` | string | required when type = text_card | not empty |
| `spec.label` | string | required when type = quick_jump | not empty |
| `spec.target` | string | required when type = quick_jump | not empty |
| `spec.items` | array | required when type = todobox | `[{text: string, done: boolean}]` |
| `spec.entries` | array | required when type = git_display | `[{hash: string, msg: string, time: string}]` |
| `spec.diff` | string | optional when type = git_display | — |
| `spec.chart` | object | required when type = data_chart | `{kind: "line"/"bar"/"pie", labels: string[], series: [{name, data}]}` |

**return**

| Case | Return |
|------|------|
| Created | `{"ok": true, "rendered": "<spec.id>"}` |
| Updated | `{"ok": true, "updated": "<spec.id>"}` |
| Closed | `{"ok": true, "closed": "<spec.id>"}` |
| Invalid id | `{"ok": false, "error": "spec.id '<id>' 不合法：…"}` |
| Missing required field | `{"ok": false, "error": "<type> 缺少必填字段：…"}` |
| Invalid type | `{"ok": false, "error": "未知 Component type：'<type>'"}` |

**effect**: `RenderComponent(spec)` (create/update) — frontend creates/updates an independent Tauri window to render the Card

## fetch_terminal

Reads the current Terminal Content of a specified instance on demand.

| Parameter | Type | Required | Validation |
|------|------|------|------|
| `instance` | string | ✓ | non-empty string |
| `vd_switch` | boolean | ✓ | `true` / `false` |

**return**

| Case | Return |
|------|------|
| Content read | `{"ok": true, "instance": "<inst>", "content": "<全文>"}` |
| instance empty | `{"ok": false, "error": "instance 必填"}` |
| vd_switch missing | `{"ok": false, "error": "vd_switch 必填（…）"}` |
| Unreadable | `{"ok": false, "error": "读不到 <inst>…}"}` |

**effect**: no injection — the result is returned directly to Context as a `tool` message (read side effect: raw text archived + in-memory prev updated)

## set_autonomy

Overrides Autonomy's expression/movement. Falls back to default after ttlMs.

| Parameter | Type | Required | Validation |
|------|------|------|------|
| `key` | string | | unique key in the union of `kaomoji.system` / `kaomoji.user` (such as `idle`/`notify`/`processing`) |
| `motion` | string | | `still` / `float` / `bounce` / `shake` |
| `ttlMs` | integer | | ≥ 0; cannot be passed together with `once: true` |
| `once` | boolean | | when `true`, takes duration automatically from the motion's `MotionDef.durationMs`; cannot be passed together with `ttlMs` |

**return**

| Case | Return |
|------|------|
| Validation passed | `{"ok": true}` |
| Invalid key | `{"ok": false, "error": "无效 key：'<v>'"}` |
| Invalid motion | `{"ok": false, "error": "motion '<v>' 不合法，合法值：still/float/bounce/shake"}` |
| `once` and `ttlMs` passed together | `{"ok": false, "error": "once 与 ttlMs 不能同时传（…）"}` |

**effect**: `SetAutonomy { face, motion, ttl_ms }` — Autonomy engine overrides expression/movement

## Memory and Cron

The following tools access Harness's Memory / Cron persistence concepts; they do not let the Agent directly read or write arbitrary files or execute arbitrary commands. For the data boundaries of Memory / Cron see `docs/harness.md`.

| tool | Contract |
|---|---|
| `read_memory` | reads the persistent understanding Markdown; name omitted = reads index.md navigation. Full schema in `docs/memory.md` |
| `write_memory` | creates or fully replaces one fragmented memory; must include a description, subject to the per-file length cap. Full schema in `docs/memory.md` |
| `cron_create` | creates a Harness persistent plan (schedule: at/every_ms + message). Full schema in `docs/cron.md` |
| `cron_delete` | deletes a persistent plan (id). Full schema in `docs/cron.md` |
| `sleep` | delays the tool result via the same Harness scheduler (0–300000ms), then continues the planned tool sequence. Full schema in `docs/cron.md` |

## edit_config

Single Config tool. All behaviors are explicitly distinguished by the required `action`; mode is never switched via missing parameters, empty values, or failed writes. For Config projection, visible paths, `query` snapshots, and the unified write pipeline see docs/config.md.

| action | Parameters | Semantics |
|---|---|---|
| `grep` | `pattern: string` | searches with Rust regex over the path and Chinese desc of LLM-visible nodes; returns candidates sorted by path, does not return value |
| `query` | `path: string`, `view?: "children" / "object"` | queries a node by exact path; leaves carry value, containers expand one level by default, `view=object` reads the complete container value |
| `update` | `path: string`, `value: any` | modifies an exact path; requires a complete query truth snapshot from an earlier response that is still valid |

**return**

| Case | Return |
|---|---|
| `grep` no match | `{"ok": true, "matches": []}` |
| `query` success | `{"ok": true, "node": {…}, "children": […]}` |
| `update` success (hot field) | `{"ok": true, "path": "…", "msg": "已生效"}` |
| `update` success (cold field) | `{"ok": true, "path": "…", "restartRequired": ["…"], "msg": "已保存，重启应用后生效"}` |
| Illegal action / parameter / regex / path | `{"ok": false, "error": "…"}` |
| query snapshot missing or invalid | `{"ok": false, "error": "缺少已读快照：未读取目标当前值。请先 query"}` |
| Validation failed or read-only degradation | `{"ok": false, "error": "…"}` |

**effect**: `ConfigChanged` on successful update — frontend reloads config
