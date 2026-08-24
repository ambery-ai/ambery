# Terminal Adapter 设计

[English](terminal-adapter.md) | 中文

终端访问抽象：分层模型——L1（hook + 查找）→ M1 → L2（综合查询管线）→ M2 → L3（可见可调）→ M3 → agenttool。多终端兼容 = 每终端提供 L1（传输/查找），查询策略独立成 L2。

> 概念定位见 `concepts.md` §14（Terminal Adapter）。

## 分层模型

```
agenttool —— agent 拿 M3
  ▲ M3 = 查询结果（实例状态 + 命中 + 可选内容 + 参数）
L3 · 可见可调
  ▲ M2（最终命中结果）
L2 · 综合查询（管线，可组合，用户可重写）
  ├─ stage 1 · 确凿条件过滤 ──► M2
  ├─ stage 2 · 歧义打分     ──► M2
  └─ … 用户可插自己的阶段    ──► M2
  ▲ M1 = { tab 属性, hook 记录 }
L1 · hook + 查找本身
```

### L1 · 传输 + 枚举（终端载体语言，没有 codecli）

每终端一个实现的**最小契约**——协议要求尽可能少，所有术语都是终端载体语言，绝不出现 Code CLI：

- `enumerate() -> Vec<TabInfo>` — 遍历该终端的 tab/pane，返回载体属性（id / title / cwd / command / focused / …）；这是 M1 的 tab 属性部分，也是发现（启动扫描、对账）的基础。
- `read(tab_id) -> ReadOutcome` — 按不透明 id 读一个载体的文字：
  - `Content(text)` — 读到 → 观测：活着；
  - `Gone` — **确证**不存在（reader 能验证该 id 已无）→ 观测：死亡（强证据）；
  - `Error` — 瞬时失败 → 无观测，重试；**绝不当作死亡**。

L1 **不认识实例**：没有 `locate(实例名)`、没有 marker 匹配、没有 sid8 / project / status。这些都在上层——M1 的 hook 记录部分来自 hook 事件通道；Code CLI ↔ tab 的 **JOIN** 是 L2 综合查询管线的职责（按 marker / 属性匹配、歧义打分）。

### M1 · 契约（纯数据）

`{ tab 属性, hook 记录 }`。查询的输入数据，全量保留（tab 的 id/title/cwd/command/focused/…；hook 的 sid8/project/status/…），一个不丢。tab 属性由 L1 的 `enumerate` 产出；hook 记录经后端 hook 事件通道到达。

### L2 · 综合查询（管线，可组合，用户可重写）

一条可组合管线，内部套多个阶段（确凿条件过滤 → 歧义打分 → …）。用户可插入 / 重写自己的阶段（seam，为用户插件）。**每个阶段边界产出一次 M2**。

### M2 · 匹配结果

`命中 / 歧义（候选）/ 没找到`。命中 = Found(tab)；歧义与没找到是失败路径（错误通道），不进正常结果。

### L3 · 可见可调

结果呈现 + 参数调整（歧义时用户可看到候选、修正匹配）。产出 **M3**。

### M3 · 查询结果

实例状态 + 匹配结果 + 可选内容 + 可调参数。

### agenttool

agent 拿 M3 的工具。

## 原则

- **可插件化（seam）** — 每层边界是 provider/consumer 契约（一个 seam），adapter 可插件化：用户新增终端类型与查询阶段，不碰 core。
- **职责不 code-cli 专用化** — adapter 抽象服务于闭环的读取与枚举；未来终端可能服务 code cli 之外的多用途，抽象不得写成 code cli 专用。

## 实现（L1 传输 / 查找 provider）

| adapter | 形态 | 访问方式 | 平台 |
|---|---|---|---|
| **WtAdapter** | 独立 C# 进程 | stdio JSONL 调 C#；UIA（CASCADIA/TermControl）定位+读取 | Windows |
| **ZellijAdapter** | 进程内（Rust 直调 CLI） | `zellij action` 命令（list-panes / dump-screen / …） | 跨平台 |
| **MapAdapter** | 进程内（core 内建） | 共享 map（case-runner 的 terminal/terminal_gone 剧情源） | 跨平台 |
| **Composite** | 进程内（core 内建） | 多 adapter 分发 | 跨平台 |

WtAdapter 保持独立进程形态——UIA 读取依赖 .NET 程序集，Rust 无法直接接 UIA TextPattern，故独立 exe。ZellijAdapter 调 CLI 即可，Rust 原生执行，无独立进程。

定位的查询策略在 **L2**，不硬编码在 adapter：`find_pane` 的 `title.contains(实例名)` 假设 zellij title 带 marker（`project·sid8`），实测真实 pane title 是 `◐ ambery` / `✳ agent-team`（spinner + 项目名，无 sid8）→ 定位失配。查询策略改走 L2 综合管线。

## Config 字段

每 adapter 一个布尔开关：

```text
terminal.adapter_wt: bool      // 启用 wt 适配器
terminal.adapter_zellij: bool  // 启用 zellij 适配器
// 未列出的 adapter 默认 false；全 false = 无终端访问，Hook 驱动核心体验仍可用
```

WtAdapter 路径沿用约定（env `AMBERY_SIDECAR` > 仓库约定路径）；ZellijAdapter 用默认会话。

