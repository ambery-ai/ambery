# Observability（可观测性基座）

[English](observability.md) | 中文

> 概念：所有概念模块可观测，且由**编译期强制**保证覆盖（docs/case-runner.md §可观测体系）。
> 本文定义机制：trait / derive / 覆盖断言 / skip 声明；observe 输出形态与求值系统见
> `docs/case-runner.md` / `docs/case-eval-system.md`。

## 原则

> **所有模块可观测，编译期强制**——新增概念模块时必须声明其可观测性，否则编译失败；
> 覆盖不靠手写抽查（effects 曾因此漏网）。

> **全量挂载于可选编译单元**——可观测机制只在观测构建配置下启用，生产构建零影响。

> **机制最小**——派生机制只做覆盖断言，不生成组装；含派生项的组装保持手写（派生项与
> 模块非一一对应，生成反而失真）。

## 问题

**Harness 加一个字段，要么实现 Observable，要么显式声明跳过并写理由，否则 E0277**——覆盖由编译期强制，不靠手写抽查。

## 机制

### trait Observable（模块投影）

```rust
/// 可观测模块（core/src/observe.rs，cfg case-runner）
pub trait Observable {
    /// 快照投影类型（值语义，observe step 直接消费）
    type Snapshot;
    fn observe(&self) -> Self::Snapshot;
}
```

每个概念模块实现自己的投影：Queue → `Vec<QueueInput>`、Context → `Vec<MessageSnapshot>`、
EventBuffer → `Vec<String>`、`Vec<AgentEntry>` → `Vec<AgentSnapshot>`、
`Option<Usage>` → `Option<Usage>`、Memory → `Vec<MemoryNoteSnapshot>`、
CronScheduler → `Vec<CronSnapshot>`。

> filtered_content **不经 Observable**——归一全文无模块字段（不持久化，从
> terminal-content.jsonl 原文 digest 现算，docs/storage.md §filtered_content），
> 它与 panorama / answer / est_delta 同属派生项，由 case.rs 手写组装。

### derive Observe（聚合覆盖断言）

作用在 **Harness**（聚合体，非模块）上，生成覆盖断言方法：

```rust
impl Harness {
    fn __observe_coverage(&self) {
        fn require<T: ::ambery_core::observe::Observable>(_: &T) {}
        require(&self.queue);      // 每个非 skip 字段一行
        require(&self.context);
        // ...
    }
}
```

- 字段类型未实现 `Observable` → **E0277**，报错位置指向 derive 处（该字段）。
- 显式跳过：`#[observe(skip = "理由")]`——理由是必填字符串，review 可见、grep 可查。
- derive 宏名 `Observe`（聚合断言）与 trait 名 `Observable`（模块投影）刻意区分。
- 宏展开内引用 `::ambery_core::observe::Observable`；core 内用
  `extern crate self as ambery_core;` 别名让该路径在 crate 内外都解析（serde 同款手法）。

### Harness 字段覆盖表

| 字段 | 处置 | 理由 |
|------|------|------|
| queue | Observable | 概念 §10c |
| context | Observable | 概念 §10b |
| event_buffer | Observable | 概念 §10d |
| agents | Observable | 概念 §9 |
| last_usage | Observable | usage 真值 |
| last_head | skip | 装配留痕（head diff 审计），非概念模块 |
| last_usage_msg_len | skip | est 增量推导基准（派生数据，经 context_est_delta 现算观测） |
| last_usage_ts | skip | usage 行 ts 锚点（派生数据，随 last_usage 经 usage 项同步观测） |
| memory | Observable | §10f 持久化理解 buffer；observe 为 index 摘要（name / description / 条数），不默认展开正文 |
| cron | Observable | §10g 持久化计划与延时调度；observe 为计划投影（id / schedule / message / next_due），不含 sleep waiter |
| cards | Observable | Card 注册表（components.md §Card 文件）；observe 为注册表投影（id/typ/title/created/user_closed/layout 摘要），不展开 component |
| store | skip | JSONL 持久化句柄（机制非概念） |
| config_dir | skip | 路径（机制非概念） |

> skip 不是豁免原则的漏洞：它把「未观测」变成显式声明。所有概念模块都必须实现
> Observable；具体 observe 输出可以后定，但不因此跳过覆盖约束。skip 只用于机制字段。

## 验收

- `observe.rs` 模块文档含 `compile_fail,E0277` doctest：derive 一个含未实现 Observable
  字段的 struct → 编译失败且错误码恰为 E0277（零依赖，cargo test 内建）。
- 正向：`case::observe()` 输出与投影契约逐字段一致。

## 明确不做

- **不生成 CaseObserve 组装**：panorama（agents 派生）/ answer（context 派生）/
  context_est_delta（usage 落点派生）与模块非一一对应，手写组装保持可读。
- **不展开 Memory 正文或 sleep waiter**：Memory observe 只给 index 摘要，避免将长期
  理解全文摊入 case 输出；Cron observe 只给持久化计划投影，sleep waiter 是进程内、
  非持久化机制。二者都必须实现 Observable，具体字段以这里的摘要契约为准。
- **effects 的穷尽 match 投影不在本基座**：属 Effect 动作流模块（docs/storage.md
  §effect.jsonl），落地时经同一 Observable 机制接入。
