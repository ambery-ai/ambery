# Docs Spec（docs 目录约束）

[English](docs-spec.md) | 中文

> 本文档约束 `docs/*.md` 与 `concepts.md` 的职责边界与内容准入。它只管这两处；`spec.md`（技术栈）、`reports/`（调研）各归各管，不在本文档范围。所有 `docs/*.md` 与 `concepts.md` 改动动手前必须先通读本文件。

## 职责地图

每个 `docs/*.md` 一份职责。分组是阅读组织，不是边界强制——真正的边界是每行"一句话职责"。

### 核心运行

- `harness.md` — Harness 数据模型、注入规则、触发模型与 JSONL 存储格式
- `agent-loop.md` — LLM 抽象、Tool Set 协议与 mock hook 契约
- `llm-setup.md` — 首启 LLM 配置引导（未配置默认、引导 modal、key 输入、连通测试）
- `autonomy.md` — 表情 Autonomy 的表达式模型、默认映射与覆盖语义
- `filter.md` — 终端文本过滤策略与结构理解数据类型
- `debug-agent.md` — DebugAgent 纯 mock 与 debug CLI
- `toolset.md` — pet 可调用的九个 function definitions 参数 schema
- `agent-assistance.md` — Agent 工作监督与协作助手的能力边界
- `capability-evaluation-project.md` — 能力拆分为可重复评估项目的体系
- `effort.md` — effort 思考预算：领域层统一档位与 provider 翻译

### 存储与配置

- `storage.md` — Storage 目录布局、文件语义、记录格式与生命周期
- `config.md` — config.json + AGENTS.md 模型、migration、reconcile、统一修改管道
- `memory.md` — Memory Workspace 目录模型、read_memory / write_memory 契约

### 调度

- `timer.md` — Timer 兜底扫描调度、错峰算法与扫描动作应用点
- `cron.md` — Cron 任务模型、持久化格式与三个调度 tool 契约

### 终端访问

- `terminal-adapter.md` — 终端访问抽象：定位/读取/解除定位接口、各终端实现与 config 字段
- `sidecar.md` — WtAdapter 进程协议（wt 终端适配器的独立进程：stdio JSONL、命令集与生命周期）

### 跨平台

- `platform-primitives.md` — 平台特定能力抽象组（虚拟桌面切换等 OS 层能力）：接口与各平台实现

### 通信与协议

- `hook.md` — 真实 Claude Code hook 契约：事件分层、marker 定位、启动扫描、安装
- `core-server.md` — 内嵌薄 HTTP server：仅绑 127.0.0.1 承载外部 hook 接入
- `streaming.md` — LLM 回复流式增量推送契约
- `effect-reporting.md` — Tauri 运行时动作 effect 上报：动作层、通道、kind/payload
- `errors.md` — 错误呈现模型：错误即通知、气泡 / banner 出口、记录与呈现分离

### 前端与窗口

- `view.md` — View 物理实现、交互细节与 Config 字段
- `chat-panel.md` — Chat Panel 唤出/关闭、布局与消息渲染规则
- `components.md` — Component 调用协议、生命周期事件、方位几何
- `multi-window.md` — 多窗口方案设计
- `tauri-shell.md` — Tauri 壳形态与跨平台 UIA 边界
- `window-positioning.md` — 窗口方位布局引擎
- `window-follow.md` — 窗口跟随坐标系、职责分层与状态语义
- `pet-window-size.md` — pet 窗口尺寸公式与原则
- `theme.md` — 主题/配色表与其 Config 设施

### 国际化

- `i18n.md` — UI 与 Harness 两个独立语言偏好

### 处理流与总览

- `processing-flow.md` — 主处理流程 ASCII 图（每步写什么日志）
- `module-storage-flow.md` — 代码模块分层 + 处理流图
- `concrete-insight.md` — 真实数据 + 图演示概念链路

### 评估工具

- `case-runner.md` — Storage 快照回归与概念观测基础设施
- `case-eval-system.md` — case 表达式求值系统
- `observability.md` — 可观测性基座：编译期强制所有概念模块可观测
- `tools.md` — 开发工具集合：tools/ 目录脚本工具与 core 独立 bin 工具（locate / run-vite / ambery-activity）

### 路线图

- `post-0.1.0.md` — 0.1.0 后能力路线图：每个后续能力一段简单陈述

## 文档分布

契约文档在仓库中的位置：

- `docs/` 是 contract / 机制 / 协议文档的唯一家园，每篇都在上方职责地图登记。`packages/*/` 只放 `spec.md` 对（技术选型、依赖边界、取舍）与代码——不放契约文档。
- `docs/` 默认平铺。子文件夹只作为 package 结构的镜像存在：种类级文档入种类文件夹，leaf 级文档入其下第二层文件夹。跨种类的文档（伞契约、跨种类契约）留在 `docs/` 根。
- 引用一律用仓根路径形式（`docs/terminal/wt/sidecar.md`）；同文件夹内允许裸文件名。

## 通用原则

以下内容**禁止**写进普通 `docs/*.md`，各有专属载体：

- **版本信息**——docs 不写任何版本号或版本范围（如"X 属 0.1.0 前/后"）。版本边界由统一发布规划定义；单个能力文档不临时自定版本归属。
- **状态标注**——docs 不写易变的状态（当前契约 / 待落地 / 未决等）。被取代的历史方案删除或在原文标注历史，不以维护中的状态字段表达。
- **不引用内部 issue**——docs 不引用内部问题编号（#N、issue-xxx、issues #N）：问题跟踪归 `dev/issues.md`，文档只陈述当前状态与契约，不把内部问题编号当证据或锚点；外部上游引用（如上游 issue / discussion 编号）允许。
- **调研与论证过程**——该进 `reports/`，docs 只记录已收敛的结论；调研的来龙去脉不属于设计契约。
- **单次会话的过程回顾**（grill 回顾等）——该进 `drafts/`，不是设计契约。
- **待落地/未决实现清单**——执行项进开发 ticket（本轮修复），未来项进 `docs/post-0.1.0.md`（路线图）；不沉淀进文档冒充实现依据，也不在 `dev/issues.md` 平铺公开未决。
- **后续能力**——0.1.0 后的新能力统一写入 `docs/post-0.1.0.md`，只作路线图、每项一段简单陈述；正式设计开启时再拆独立文档。

以上通用原则同样约束 `concepts.md`（领域概念文档）。

## 概念文档规范（concepts.md）

`concepts.md` 是领域概念的一等文档，受上述通用原则约束。概念条目的组成：

- **定位**——概念是什么，一句话讲清。
- **边界与关系**——与相关概念的界限、被谁使用、在整体中的位置，直接陈述。
- **可实例化**——概念应当能落成可实例化的类型（trait / enum / struct），不是抽象名词。
- 实现细节、协议、config 字段、命令集归对应 `docs/*.md`；**概念不引用设计文档**——概念条目自包含定义，不指向 `docs/*.md`；引用方向为 docs → concepts（设计文档可引概念，概念不引设计文档）。

概念与设计的边界：concepts.md 回答"是什么、边界在哪"，docs/*.md 回答"怎么实现、接口长什么样"。概念改动同样先通读本文件。

**例子要求**：例子写在概念文档里，每个例子分析一条完整过程并标注其中每个概念代表什么；所有概念至少被一个例子覆盖，一个例子尽可能多地覆盖概念且至少覆盖三个，例子数量与概念数量不对等（一条例子覆盖多概念，例子数可以少于概念数）。
