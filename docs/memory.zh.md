# Memory Workspace 设计

[English](memory.md) | 中文

> 概念定义见 concepts.md §10f。本文档定 Memory Workspace 的目录模型、notes 的 read_memory / write_memory 调用契约，以及 index/AGENTS.md 的生成契约。Card 是同一工作空间中的持久工作产物，但不属于普通 Memory note，也不经这两个 tool 管理；其文件契约见 docs/components.md §Card 文件。

## 原则

> **本文档范围**——本文定义 Memory Workspace 的目录模型、notes 的两个 tool 调用契约与 index/AGENTS.md 的生成规则；Memory 的概念定位、所有权与持久化边界见 concepts.md §10f / docs/harness.md §Memory；存储布局见 docs/storage.md §Memory Workspace。

> **工作空间而非扁平根**——Memory 是持久工作空间；notes 与 cards 按产物语义分目录。扁平化不是原则：只有 notes 当前不再细分，不能阻止其他持久产物拥有自己的目录。

> **设计常量**——note 单文件长度上限、文件名与 description 上限是实现常量（本文定义值），不进 Config。

> **不做特殊规则，用语义明确行为**——不为删除引入隐式空值语义；不以缺参切换读写模式（`name` 省略只有一种明示语义：读 index 导航）。

## 文件模型

`storage/memory/` 是唯一 Memory Workspace 根：

```text
memory/
├─ AGENTS.md          ← 工作空间导航与纪律（默认只读）
├─ index.md           ← 自动汇总 notes 的 frontmatter description（默认只读）
├─ notes/             ← Agent 的长期理解；当前不再细分目录
│  └─ <name>.md       ← 普通 note
└─ cards/             ← 持久 Component / 工作产物
   └─ <id>.card.json  ← 文件即 Card；完整 JSON（内容与 Surface 状态同位）
```

- `notes/`：普通 Memory note，短小、碎片化；`read_memory` / `write_memory` 只管理此目录。
- `cards/`：持久 Component 工作产物；每张 Card 是一个完整 JSON 文件，note 可用 `cards/<id>.card.json` 稳定相对路径引用。它不被普通 note 索引扫描，也不经 `read_memory` / `write_memory` 管理。
- `index.md`：自动汇总 `notes/` 内普通 note 的名称与 frontmatter `description`。
- `AGENTS.md`：整个 Memory Workspace 的导航信息；与 Config 域身份提示词 `AGENTS.md` 不是同一文件。

普通 note 以 YAML frontmatter metadata 开头；当前唯一已定义字段是必填 `description`：

```md
---
description: 用户的工作偏好与协作方式
---

- 不擅自提交
```

frontmatter 是文件原文的一部分；`read_memory` 返回全文时也返回它。其他 metadata 字段不定义、不由 `write_memory` 写入。note 文件名 grammar：`^[a-z][a-z0-9_-]*$` 且 ≤ 64 字符；名称只作用于 `notes/`，不再承担整个工作空间的扁平化约束。保留名 `index` 与 `AGENTS`：可读、不可写。

#### ⟡ 一致性剖析

notes 与 cards 同属持久工作空间，因此共享根目录、跨重启边界与 `AGENTS.md` 导航；但它们不能被混成一种文件。note 是 Agent 的长期理解，受 frontmatter、description、index 与 `read_memory` / `write_memory` 约束；Card 是结构化工作产物，文件即 Component 与 Surface 状态，不参与 note 索引也不复用 note tool。这样 note 能以稳定相对路径关联 Card，又不会把 Card 错当成可随意全文覆盖的记忆。

## read_memory

读取一条记忆的全文。

| 参数 | 类型 | 必填 | 校验 |
|---|---|---|---|
| `name` | string | | 省略 = 读 `index.md` 导航首页；否则为普通 note 名或保留名 |

**return**

| 情况 | 返回 |
|---|---|
| 成功 | `{"ok": true, "name": "<name>", "content": "<全文>"}`（读 index 时 `name` 为 `"index"`） |
| 不存在 | `{"ok": false, "error": "记忆 '<name>' 不存在（先 read_memory() 看 index）"}` |
| 非法名 | `{"ok": false, "error": "名称 '<name>' 不合法：…"}` |

## write_memory

新建或完整替换一条普通 note；无局部 patch。

| 参数 | 类型 | 必填 | 校验 |
|---|---|---|---|
| `name` | string | ✓ | 文件名 grammar；拒绝 `index` / `AGENTS`（默认只读） |
| `content` | string | ✓ | UTF-8 字节数 ≤ 4096（碎片化记忆） |
| `description` | string | ✓ | 非空、单行、不含 `\|`，≤ 80 字符；写入文件 frontmatter 的 `description`，并进入 index.md |

**return**

| 情况 | 返回 |
|---|---|
| 成功 | `{"ok": true, "name": "<name>"}` |
| 缺/错参数 | `{"ok": false, "error": "…"}` |

**effect**：无副作用广播（Memory 是后端数据）；写入成功后 `index.md` 自动全量重生成。

## index.md 契约

每次 write_memory 成功后全量重生成（手写 index.md 会被下一次 write 覆盖——自动汇总语义）：

```md
# Memory Index

| 名称 | 描述 |
|---|---|
| [work-preferences](notes/work-preferences.md) | 用户的工作偏好与协作方式 |
```

- 按名称字典序排列
- description 存于文件开头 YAML frontmatter 的 `description` 字段（与正文同文件不漂移；read 返回的全文含 frontmatter）
- frontmatter 只定义 `description`：必须是单行标量，不得出现未定义 metadata 字段；格式不合法的普通 note 不进入 index.md
- 外部直接增删文件后，index.md 在下一次 write 时自动收敛到实际文件集

## AGENTS.md（Memory 根）契约

bootstrap：Harness 启动时若 Memory 根或其中 `AGENTS.md` 不存在，则创建目录与默认内容（目录性质、index.md 用途、读写规则的索引导航说明）。默认只读（agent 不可写）；用户与后端可直接编辑管理——与 Config 域身份提示词的热读路径互不相关。

## 删除语义

当前无删除 tool：普通 note 通过同名覆盖演进；确需删除由用户或后端直接管理 `notes/` 文件，`index.md` 在下一次 write 时自动收敛。Card 的 dismiss / 删除语义由 Card 文件与 Surface 生命周期契约定义，不混入 note 的删除规则。

## 与 Context 的关系

Memory 不参与请求自动装配：agent 按需 `read_memory` 取回理解、`write_memory` 沉淀理解；读写结果经 tool result 进 Context，与所有 tool 一致。
