# Platform Primitives 设计

[English](platform-primitives.md) | 中文

平台特定能力的抽象组（不叫 adapter）：虚拟桌面切换等跨终端、跨组件复用的 OS 层能力。

> 概念定位见 `concepts.md` §15（Platform Primitives）。本文件定接口能力与各平台实现。

## 能力接口

platform-primitives 是可实例化的一类东西——落成 Rust trait，各平台一个实现：

```rust
pub trait PlatformPrimitives: Send + Sync {
    /// 切到目标窗口所在虚拟桌面（读取前置 / 跳转共用）
    fn switch_vd(&self, hwnd: i64) -> bool;
}
```

## 消费者

- **terminal-adapter**（docs/terminal/terminal-adapter.md）：读取时目标窗口不可见（cloaked）→ 切桌面后读。切换是打断性动作，须经 fetch_terminal 的 `vd_switch` 显式同意门控（docs/agents/claude/hook.md §VD 切换能力），adapter 不自动切

## 实现

| 平台 | 实现方式 | 能力 |
|---|---|---|
| Windows | COM（`IVirtualDesktopManager`） | 虚拟桌面切换 |
| （其他平台按需） | — | — |
