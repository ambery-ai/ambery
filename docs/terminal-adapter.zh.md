# Terminal Adapter 设计

[English](terminal-adapter.md) | 中文

终端访问抽象：向 Code CLI 实例提供「定位、读取、解除定位」能力的统一接口。多终端兼容 = 抽象接口 + 按终端分发实现。

> 概念定位见 `concepts.md` §14（Terminal Adapter）。本文件定接口能力、实现与 config 字段。

## 能力接口

terminal-adapter 是**可实例化的一类东西**——落成 Rust trait，各终端一个实现：

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

一个 adapter 实例对应一个终端类型（wt / zellij / …）。core 侧按 config 启用情况装配对应 adapter。

## 实现

| adapter | 形态 | 访问方式 | 平台 |
|---|---|---|---|
| **WtAdapter** | 独立 C# 进程 | stdio JSONL 调 C#；UIA（CASCADIA/TermControl）定位+读取 | Windows |
| **ZellijAdapter** | 进程内（Rust 直调 CLI） | `zellij action` 命令（list-tabs / rename-tab / query-tab-names…） | 跨平台 |
| **MapAdapter** | 进程内（core 内建） | 共享 map（case-runner 的 terminal/terminal_gone 剧情源） | 跨平台 |
| **Composite** | 进程内（core 内建） | 多 adapter 分发：locate 首中记录路由、read 回到产出 adapter、unlocate 广播 | 跨平台 |

WtAdapter 保持独立进程形态——UIA 读取依赖 .NET 程序集，Rust 无法直接接 UIA TextPattern，故独立 exe。ZellijAdapter 调 CLI 即可，Rust 原生执行，无独立进程。

### WtAdapter

定位经 UIA 扫 CASCADIA 窗口、读取经 TermControl TextPattern，stdio JSONL 协议与生命周期见 `docs/sidecar.md`。它是 terminal-adapter 的一个实现。

### ZellijAdapter

zellij 是跑在终端里的复用器（pane 层），需在底层终端之上叠加定位。实现经 `zellij action` CLI 适配（定位 marker、读 pane 内容）。

## Config 字段

每 adapter 一个布尔开关（纯开关起步，参数先用约定）：

```text
terminal.adapter_wt: bool      // 启用 wt 适配器
terminal.adapter_zellij: bool  // 启用 zellij 适配器
// 未列出的 adapter 默认 false；全 false = 无终端访问，Hook 驱动核心体验仍可用
```

WtAdapter 路径沿用约定（env `AMBERY_SIDECAR` > 仓库约定路径）；ZellijAdapter 用默认会话。

