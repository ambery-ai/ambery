# Spec — packages/terminals/zellij

[English](spec.md) | 中文

## 技术选型

- **进程内 Rust CLI 调用**（`zellij action`）：pane 列举、屏幕 dump——CLI 本身就是传输层；无 sidecar 进程。

## 架构决定

1. **无独立进程**：与 wt 不同，CLI 自身的生命周期就是隔离边界；sidecar 徒增一跳、无收益。
2. **跨平台**：zellij 可运行处即可工作（macOS/Linux）；无 UIA、无平台门控。

## 固定约束

- pane 标题携带 marker，消费方 join 据此命中（标题约定归 agents/claude 包）；join/查询策略在消费方的接入管线中，不硬编码于本包。
