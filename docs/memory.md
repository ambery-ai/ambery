# Memory Workspace Design

> See concepts.md §10f for the concept definition. This document defines the directory model of the Memory Workspace, the read_memory / write_memory call contract for notes, and the generation contract for index/AGENTS.md. Card is a persistent work artifact in the same workspace, but it is not an ordinary Memory note and is not managed through these two tools; for its file contract, see docs/components.md §Card File.

## Principles

> **Scope of this document** — this document defines the Memory Workspace's directory model, the call contract of the two note tools, and the generation rules for index/AGENTS.md; for the conceptual positioning, ownership, and persistence boundaries of Memory, see concepts.md §10f / docs/harness.md §Memory; for the storage layout, see docs/storage.md §Memory Workspace.

> **A workspace, not a flat root** — Memory is a persistent workspace; notes and cards are divided into directories by artifact semantics. Flatness is not a principle: only notes are currently not further subdivided, and that must not prevent other persistent artifacts from having their own directories.

> **Design constants** — the per-note file length limit and the file-name and description limits are implementation constants (values defined in this document), not Config.

> **No special rules; make behavior explicit through semantics** — no implicit null-value semantics are introduced for deletion; missing parameters do not switch read/write modes (omitting `name` has exactly one explicit meaning: read the index navigation).

## File Model

`storage/memory/` is the single Memory Workspace root:

```text
memory/
├─ AGENTS.md          ← 工作空间导航与纪律（默认只读）
├─ index.md           ← 自动汇总 notes 的 frontmatter description（默认只读）
├─ notes/             ← Agent 的长期理解；当前不再细分目录
│  └─ <name>.md       ← 普通 note
└─ cards/             ← 持久 Component / 工作产物
   └─ <id>.card.json  ← 文件即 Card；完整 JSON（内容与 Surface 状态同位）
```

- `notes/`: ordinary Memory notes, short and fragmentary; `read_memory` / `write_memory` manage only this directory.
- `cards/`: persistent Component work artifacts; each Card is one complete JSON file, and a note can reference it by the stable relative path `cards/<id>.card.json`. It is not scanned by the ordinary note index, nor managed through `read_memory` / `write_memory`.
- `index.md`: automatically aggregates the names and frontmatter `description` of ordinary notes under `notes/`.
- `AGENTS.md`: navigation information for the entire Memory Workspace; it is not the same file as the Config-domain identity prompt `AGENTS.md`.

An ordinary note starts with YAML frontmatter metadata; the only currently defined field is the required `description`:

```md
---
description: 用户的工作偏好与协作方式
---

- 不擅自提交
```

The frontmatter is part of the file's original text; `read_memory` returns it when returning the full text. No other metadata fields are defined, and none are written by `write_memory`. Note file-name grammar: `^[a-z][a-z0-9_-]*$` and ≤ 64 characters; the name applies only within `notes/` and no longer carries the flatness constraint for the whole workspace. Reserved names `index` and `AGENTS`: readable, not writable.

#### ⟡ Consistency Analysis

Notes and cards both belong to the persistent workspace, so they share the root directory, the restart boundary, and `AGENTS.md` navigation; but they must not be conflated into one kind of file. A note is the Agent's long-term understanding, constrained by frontmatter, description, index, and `read_memory` / `write_memory`; a Card is a structured work artifact whose file is both the Component and Surface state, and it does not participate in the note index or reuse the note tools. This lets a note associate with a Card through a stable relative path, without mistaking a Card for memory that can be freely overwritten in full.

## read_memory

Reads the full text of one memory.

| Parameter | Type | Required | Validation |
|---|---|---|---|
| `name` | string | | Omitted = read the `index.md` navigation home; otherwise an ordinary note name or a reserved name |

**Return**

| Case | Return |
|---|---|
| Success | `{"ok": true, "name": "<name>", "content": "<全文>"}` (when reading index, `name` is `"index"`) |
| Not found | `{"ok": false, "error": "记忆 '<name>' 不存在（先 read_memory() 看 index）"}` |
| Invalid name | `{"ok": false, "error": "名称 '<name>' 不合法：…"}` |

## write_memory

Creates a new ordinary note or fully replaces one; no partial patch.

| Parameter | Type | Required | Validation |
|---|---|---|---|
| `name` | string | ✓ | File-name grammar; rejects `index` / `AGENTS` (read-only by default) |
| `content` | string | ✓ | UTF-8 byte count ≤ 4096 (fragmentary memory) |
| `description` | string | ✓ | Non-empty, single-line, no `\|`, ≤ 80 characters; written to the file's frontmatter `description`, and enters index.md |

**Return**

| Case | Return |
|---|---|
| Success | `{"ok": true, "name": "<name>"}` |
| Missing/incorrect parameters | `{"ok": false, "error": "…"}` |

**Effect**: no side-effect broadcast (Memory is backend data); after a successful write, `index.md` is automatically regenerated in full.

## index.md Contract

Fully regenerated after every successful write_memory (a hand-written index.md is overwritten by the next write — automatic aggregation semantics):

```md
# Memory Index

| 名称 | 描述 |
|---|---|
| [work-preferences](notes/work-preferences.md) | 用户的工作偏好与协作方式 |
```

- Sorted by name in lexicographic order.
- The description is stored in the `description` field of the YAML frontmatter at the top of the file (kept in the same file as the body so it cannot drift; the full text returned by read includes the frontmatter).
- Frontmatter defines only `description`: it must be a single-line scalar, and undefined metadata fields must not appear; ordinary notes with invalid format do not enter index.md.
- After files are added or removed directly by external means, index.md automatically converges to the actual file set on the next write.

## AGENTS.md (Memory Root) Contract

Bootstrap: at Harness startup, if the Memory root or its `AGENTS.md` does not exist, the directory and default content are created (index navigation describing the directory's nature, the purpose of index.md, and the read/write rules). Read-only by default (agents cannot write); users and the backend can edit and manage it directly — this is unrelated to the hot-read path of the Config-domain identity prompt.

## Deletion Semantics

There is currently no deletion tool: ordinary notes evolve through same-name overwrite; when deletion is truly necessary, the user or backend directly manages the `notes/` files, and `index.md` automatically converges on the next write. Card dismiss / deletion semantics are defined by the Card file and the Surface lifecycle contract, and are not mixed into note deletion rules.

## Relationship with Context

Memory does not participate in automatic request assembly: the agent calls `read_memory` on demand to retrieve understanding and `write_memory` to persist understanding; the read/write results enter Context through tool results, the same as all tools.
