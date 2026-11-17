# Docs Spec（docs 目录约束）

[English](docs-spec.md) | 中文

> 本文档约束 `docs/*.md` 与 [concepts.zh.md](concepts.zh.md) 的职责边界与内容准入。它只管这两处；`reports/`（调研）另归其管，不在本文档范围。所有 `docs/*.md` 与 [concepts.zh.md](concepts.zh.md) 改动动手前必须先通读本文件。

## 职责地图

每个 `docs/*.md` 一份职责。分组是阅读组织，不是边界强制——真正的边界是每行"一句话职责"。

### 核心运行

- [docs/harness.zh.md](docs/harness.zh.md) — Harness 数据模型、注入规则、触发模型与 JSONL 存储格式
- [docs/agent-loop.zh.md](docs/agent-loop.zh.md) — LLM 抽象、Tool Set 协议与 mock hook 契约
- [docs/llm-setup.zh.md](docs/llm-setup.zh.md) — 首启 LLM 配置引导（未配置默认、引导 modal、key 输入、连通测试）
- [docs/autonomy.zh.md](docs/autonomy.zh.md) — 表情 Autonomy 的表达式模型、默认映射与覆盖语义
- [docs/effort.zh.md](docs/effort.zh.md) — effort 思考预算：领域层统一档位与 provider 翻译
- [docs/toolset.zh.md](docs/toolset.zh.md) — pet 可调用的九个 function definitions 参数 schema
- [docs/cron.zh.md](docs/cron.zh.md) — Cron 任务模型、持久化格式与三个调度 tool 契约
- [docs/memory.zh.md](docs/memory.zh.md) — Memory Workspace 目录模型、read_memory / write_memory 契约
- [docs/streaming.zh.md](docs/streaming.zh.md) — LLM 回复流式增量推送契约

### 存储与配置

- [docs/storage.zh.md](docs/storage.zh.md) — Storage 目录布局、文件语义、记录格式与生命周期
- [docs/config.zh.md](docs/config.zh.md) — config.json + AGENTS.md 模型、migration、reconcile、统一修改管道
- [docs/memory.zh.md](docs/memory.zh.md) — Memory Workspace 目录模型、read_memory / write_memory 契约

### 接入协议

- [docs/access-protocol.zh.md](docs/access-protocol.zh.md) — Ambery Protocol 契约：外部软件如何成为 Source，事件如何进出

#### 终端

- [docs/terminal/terminal-adapter.zh.md](docs/terminal/terminal-adapter.zh.md) — 终端访问抽象：定位/读取/解除定位接口、各终端实现、config 字段与分层接入模型（L1–L3 / M1–M3）
- [docs/terminal/wt/sidecar.zh.md](docs/terminal/wt/sidecar.zh.md) — WtAdapter 进程协议（wt 终端适配器的独立进程：stdio JSONL、命令集与生命周期）
- [docs/terminal/timer.zh.md](docs/terminal/timer.zh.md) — 兜底巡逻扫描调度、错峰算法与扫描动作应用点

#### Agent

- [docs/agents/claude/hook.zh.md](docs/agents/claude/hook.zh.md) — 真实 Claude Code hook 契约：事件分层、marker 定位、启动扫描、安装
- [docs/agents/filter.zh.md](docs/agents/filter.zh.md) — 终端文本过滤策略与结构理解数据类型

#### 宿主侧

- [docs/core-server.zh.md](docs/core-server.zh.md) — 内嵌薄 HTTP server：仅绑 127.0.0.1 承载外部 hook 接入

### 跨平台

- [docs/platform-primitives.zh.md](docs/platform-primitives.zh.md) — Ambery 自身的平台能力层（自身窗口的定位/聚焦/桌面处理）：接口与各平台实现

### 运行时上报与错误

- [docs/effect-reporting.zh.md](docs/effect-reporting.zh.md) — Tauri 运行时动作 effect 上报：动作层、通道、kind/payload
- [docs/errors.zh.md](docs/errors.zh.md) — 错误呈现模型：错误即通知、气泡 / banner 出口、记录与呈现分离

### 前端与窗口

- [docs/view.zh.md](docs/view.zh.md) — View 物理实现、交互细节与 Config 字段
- [docs/chat-panel.zh.md](docs/chat-panel.zh.md) — Chat Panel 唤出/关闭、布局与消息渲染规则
- [docs/components.zh.md](docs/components.zh.md) — Component 调用协议、生命周期事件、方位几何
- [docs/multi-window.zh.md](docs/multi-window.zh.md) — 多窗口方案设计
- [docs/tauri-shell.zh.md](docs/tauri-shell.zh.md) — Tauri 壳形态与跨平台 UIA 边界
- [docs/window-positioning.zh.md](docs/window-positioning.zh.md) — 窗口方位布局引擎
- [docs/window-follow.zh.md](docs/window-follow.zh.md) — 窗口跟随坐标系、职责分层与状态语义
- [docs/pet-window-size.zh.md](docs/pet-window-size.zh.md) — pet 窗口尺寸公式与原则
- [docs/card-window-size.zh.md](docs/card-window-size.zh.md) — Card 窗口尺寸：唯一来源、向 Card 文件的投影与建窗
- [docs/theme.zh.md](docs/theme.zh.md) — 主题/配色表与其 Config 设施
- [docs/ui-composition.zh.md](docs/ui-composition.zh.md) — UI 代码分层与组合：宿主 / Surface / widget 分层、widget 各层、样式组合、组件原则

### 国际化

- [docs/i18n.zh.md](docs/i18n.zh.md) — UI 与 Harness 两个独立语言偏好
- `docs/i18n/README.md` — 文档集的双语配对规则
- [docs/i18n/glossary.zh.md](docs/i18n/glossary.zh.md) — 双语文档对的固定译名表

### 处理流与总览

- [docs/processing-flow.zh.md](docs/processing-flow.zh.md) — 主处理流程 ASCII 图（每步写什么日志）
- [docs/module-storage-flow.zh.md](docs/module-storage-flow.zh.md) — 代码模块分层 + 处理流图
- [docs/concrete-insight.zh.md](docs/concrete-insight.zh.md) — 真实数据 + 图演示概念链路

### 评估工具

- [docs/debug-agent.zh.md](docs/debug-agent.zh.md) — DebugAgent 纯 mock 与 debug CLI
- [docs/case-runner.zh.md](docs/case-runner.zh.md) — Storage 快照回归与概念观测基础设施
- [docs/case-eval-system.zh.md](docs/case-eval-system.zh.md) — case 表达式求值系统
- [docs/observability.zh.md](docs/observability.zh.md) — 可观测性基座：编译期强制所有概念模块可观测
- [docs/development-tools.zh.md](docs/development-tools.zh.md) — 开发工具集合：tools/ 目录脚本工具与 core 独立 bin 工具（locate / run-vite / ambery-activity）
- [docs/kitchen-sink.zh.md](docs/kitchen-sink.zh.md) — 渲染层验证面：页面、渲染内容、控件与同步保证

### 能力与 benchmark

- [docs/agent-assistance.zh.md](docs/agent-assistance.zh.md) — Agent 工作监督与协作助手的能力边界

### 路线图

- [docs/roadmap.zh.md](docs/roadmap.zh.md) — 通向 0.1.0 的发布阶梯：每档主题与验收；版本边界的唯一归属
- `post-0.1.0.md` — 0.1.0 后能力路线图：每个后续能力一段简述

### 本仓工作规范

- `AGENTS.md` — 文档写作标准：文档结构、写作与链接规则、语言规则、强制流程
- `DEVELOPING.md` — 开发指南：构建、测试、跑全栈、观察 storage 与窗口、调试 LLM 失败

## 文档分布

契约文档在仓库中的位置：

- `docs/` 是 contract / 机制 / 协议文档的唯一家园，每篇都在上方职责地图登记。`packages/*/` 只放 [spec.zh.md](spec.zh.md) 对（技术选型、依赖边界、取舍）与代码——不放契约文档。
- `docs/` 留在根、不拆：接入类文档住在 `docs/`（`docs/terminal/`、`docs/agents/`、`docs/cron.md`），不进 packages。
- 同组的父文档不承担子文档的登记：组里有哪些文档由责任地图陈述一次，不在父文档引言里逐一列举。
- 引用另一篇文档用 Markdown 链接：链接文字写仓根路径（`docs/terminal/wt/sidecar.md`），目标是相对当前文件的路径；同一文件夹内，裸文件名同时充当文字与目标。

## 通用原则

以下内容**禁止**写进普通 `docs/*.md`，各有专属载体：

- **版本信息**——docs 不写任何版本号或版本范围（如"X 属 0.1.0 前/后"）。版本边界由发布阶梯（[docs/roadmap.zh.md](docs/roadmap.zh.md)）定义、且仅由它定义；单个能力文档不临时自定版本归属。
- **状态标注**——docs 不写易变的状态（当前契约 / 待落地 / 未决等）。被取代的历史方案删除或在原文标注历史，不以维护中的状态字段表达。
- **不引用内部 issue**——docs 不引用内部问题编号（#N、issue-xxx、issues #N）：文档只陈述当前状态与契约，不把内部问题编号当证据或锚点；外部上游引用（如上游 issue / discussion 编号）允许。
- **调研与论证过程**——该进 `reports/`，docs 只记录已收敛的结论；调研的来龙去脉不属于设计契约。
- **单次会话的过程回顾**（grill 回顾等）——该进 `drafts/`，不是设计契约。
- **待落地/未决实现清单**——执行项进开发 ticket（本轮修复），未来项进 `docs/post-0.1.0.md`（路线图）；不沉淀进文档冒充实现依据。
- **后续能力**——0.1.0 后的新能力统一写入 `docs/post-0.1.0.md`，只作路线图、每项一段简单陈述；正式设计开启时再拆独立文档。

以上通用原则同样约束 [concepts.zh.md](concepts.zh.md)（领域概念文档）。

## 原则段

契约文档用 `## 原则` 段声明其长期有效的规则，位置在文档顶部的范围注记之后、机制各节之前。这一段不是必需的：规则已由文档自身的数据模型、字段表、接口或图示承载的文档（格式、坐标契约、流程图）不加此段，缺这一段不算缺口。

- 一条原则一个引用段，形式为 `> **<名字>** —— <规则>`；原则之间空一行。
- 第一条原则是本文的范围：本文定义什么，不定义什么、归哪份文档。
- 原则只写长期规则与边界，不写机制步骤；需要细节的原则指向拥有该细节的节或文档。
- 原则以规则命名，不用编号、版本或状态。

## 概念文档规范（concepts.md）

[concepts.zh.md](concepts.zh.md) 是领域概念的一等文档，受上述通用原则约束。概念条目的组成：

- **定位**——概念是什么，一句话讲清。
- **边界与关系**——与相关概念的界限、被谁使用、在整体中的位置，直接陈述。
- **可实例化**——概念应当能落成可实例化的类型（trait / enum / struct），不是抽象名词。
- 实现细节、协议、config 字段、命令集归对应 `docs/*.md`；**概念不引用设计文档**——概念条目自包含定义，不指向 `docs/*.md`；引用方向为 docs → concepts（设计文档可引概念，概念不引设计文档）。

概念与设计的边界：concepts.md 回答"是什么、边界在哪"，docs/*.md 回答"怎么实现、接口长什么样"。概念改动同样先通读本文件。

**例子要求**：例子写在概念文档里，每个例子分析一条完整过程并标注其中每个概念代表什么；所有概念至少被一个例子覆盖，一个例子尽可能多地覆盖概念且至少覆盖三个，例子数量与概念数量不对等（一条例子覆盖多概念，例子数可以少于概念数）。
