# Terminal Adapter Design

English | [中文](terminal-adapter.zh.md)

Terminal access abstraction: a unified interface that gives Code CLI instances the ability to "locate, read, unlocate". Multi-terminal compatibility = abstract interface + per-terminal dispatch implementations.

> See `concepts.md` §14 (Terminal Adapter) for the concept positioning. This file defines the interface capabilities, implementations, and config fields.

## Capability interface

terminal-adapter is **an instantiable kind of thing** — realized as a Rust trait with one implementation per terminal:

```rust
pub trait TerminalAdapter: Send + Sync {
    /// 定位：instance → 它在终端会话中的位置（TabRef）
    fn locate(&self, inst: &str) -> Option<TabRef>;
    /// 读取：读该位置的终端文字
    fn read(&self, tab: &TabRef) -> Option<String>;
    /// 解除定位：终止 instance 与位置之间的定位关系（instance 会话结束/判死）
    fn unlocate(&self, inst: &str);
}
```

An adapter instance corresponds to one terminal type (wt / zellij / …). The core side assembles the corresponding adapters according to config enablement.

## Implementations

| adapter | form | access | platform |
|---|---|---|---|
| **WtAdapter** | standalone C# process | stdio JSONL calls into C#; UIA (CASCADIA/TermControl) locate + read | Windows |
| **ZellijAdapter** | in-process (Rust calls CLI directly) | `zellij action` commands (list-tabs / rename-tab / query-tab-names…) | cross-platform |
| **MapAdapter** | in-process (built into core) | shared map (the terminal/terminal_gone scenario source for case-runner) | cross-platform |
| **Composite** | in-process (built into core) | multi-adapter dispatch: locate routes by first hit, read returns to the producing adapter, unlocate broadcasts | cross-platform |

WtAdapter keeps its standalone process form — UIA reading depends on .NET assemblies, and Rust cannot directly consume the UIA TextPattern, hence a separate exe. ZellijAdapter only needs to call the CLI, executes natively in Rust, and has no separate process.

### WtAdapter

Locating scans CASCADIA windows via UIA, reading goes through the TermControl TextPattern; for the stdio JSONL protocol and lifecycle see `docs/sidecar.md`. It is one implementation of terminal-adapter.

### ZellijAdapter

zellij is a multiplexer running inside the terminal (pane layer) and needs positioning layered on top of the underlying terminal. The implementation adapts via the `zellij action` CLI (locate markers, read pane content).

## Config fields

One boolean switch per adapter (pure switch to start; parameters use conventions for now):

```text
terminal.adapter_wt: bool      // 启用 wt 适配器
terminal.adapter_zellij: bool  // 启用 zellij 适配器
// 未列出的 adapter 默认 false；全 false = 无终端访问，Hook 驱动核心体验仍可用
```

WtAdapter uses the conventional path (env `AMBERY_SIDECAR` > repo conventional path); ZellijAdapter uses the default session.
