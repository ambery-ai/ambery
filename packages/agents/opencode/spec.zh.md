# Spec — packages/agents/opencode

[English](spec.md) | 中文

## 技术选型

- **Filter 模块**（`filter/opencode.rs` 迁入）：噪音清单、分块切分与折行合并参数，待真实样本收敛。

## 架构决定

1. **Hook 形态缓定**：opencode 的推送通道与 Claude Code 不同；形态由真实 opencode 行为决定，不从 claude 包臆测。

## 固定约束

- 与所有 agent 叶相同的叶契约：经 `ambery-terminal-lib` 注册、三态读、hook 载荷不带内容。
