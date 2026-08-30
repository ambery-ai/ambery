# Spec — packages/agents/claude

[English](spec.md) | 中文

## 技术选型

- **`"command"` 类型 hook 脚本**（Windows PowerShell / macOS shell）：读 stdin JSON → 输出 sessionTitle（marker 锚）→ fire-and-forget POST 到宿主本地端口。除 Claude Code `"command"` 类型所需外不引入解释器依赖。
- **Filter 模块**（`filter/claude.rs` 迁入）：内容规则取自真实 Claude Code 终端样本。

## 架构决定

1. **Hook 是本包的推送通道**：五个生命周期事件（SessionStart / UserPromptSubmit / Stop / SessionEnd / Notification）；其余 30+ Claude Code 事件保持保留。
2. **身份 = session_id**：sid8（前 8 位）是实例身份；同名不同命——重开同项目即新生命周期。marker 前缀（`<project>·<sid8>`）不可变；描述部分可演进。
3. **安装随本包交付**：全局 hook 配置项（~/.claude/settings.json）由本包携带并安装（平台特定脚本）；首启安装流程是它的职责。

## 固定约束

- 读通道是内容的唯一来源（hook 载荷不带内容）——本包绝不绕过宿主的读契约。
