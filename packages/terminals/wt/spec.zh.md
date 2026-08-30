# Spec — packages/terminals/wt

[English](spec.md) | 中文

## 技术选型

- **C#（.NET）sidecar**（`ambery-uia-sidecar`）：UIA 枚举、tab 定位、TermControl 全文读取。
- 保持 C#：wt 读取路径已在 C# 端到端验证；用 Rust 重写等于为不可见收益重新购买验证。C# 也有官方 MCP SDK，协议迁移无需重写。
- 打包：self-contained win-x64（非单文件），用户机器无需 .NET 运行时；以 Tauri externalBin 分发。

## 架构决定

1. **独立进程**：sidecar 是独立 exe，首次请求时惰性启动；死即丢弃，下次请求重新拉起。崩溃隔离是目的。
2. **stdio JSONL 协议**：请求/响应逐行；命令集（list_windows / read_tab / switch_tab）是本包与宿主的私有契约，直至被 Ambery Protocol 取代。
3. **可见性先于切换**：WT 开启「在所有桌面显示窗口」后一切 UIA 可读，无需切桌面；`vd_switch` 是显式同意的回退，永不自动。

## 固定约束

- 仅 Windows 的包；其他平台上宿主的 Option 链自然降级（无 sidecar = 读通道回退 Context）。
