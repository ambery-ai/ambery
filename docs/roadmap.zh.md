# Roadmap

[English](roadmap.md) | 中文

> - **本文档范围**——本文定义通向 0.1.0 的发布阶梯：每个 alpha 承载的主题与收口验收。日常推进留在开发 ticket，0.1.0 之后的能力归 [post-0.1.0.zh.md](post-0.1.0.zh.md)。

通向 0.1.0 的发布阶梯：从第一个打包快照起，每个 alpha 承载一个主题，验收成立即收口。日常推进留在开发 ticket，不在此处；0.1.0 之后的能力见 [post-0.1.0.zh.md](post-0.1.0.zh.md)。

## 主题

### alpha1 · Surface 重建

前端落进新的窗口组合：Svelte 窗口组件架在 widget 层级上，主题 token 是唯一样式源头，渲染缺陷清零。验收：窗口与卡片在桌面上行为稳定。

### alpha2 · Ambery Protocol

协议在 Claude Code 之外成为现实：外部软件经 MCP 以 Source Host 身份接入，Ambery 既消费也暴露 MCP，第二个原生结构化的 Source Host 端到端跑通。验收：来自该 Source Host 的会话抵达桌宠循环——打扰、卡片、追问。

### alpha3 · Memory 与 Session

重启之后仍在的记忆：Session 身份跨天延续、Context 跨天携带、压缩保留可恢复的细节、历史可查询、摘要自动落进 Memory。验收：桌宠记得昨天。

### alpha4 · 续航

长时间运行的可靠性：hook 投递无静默丢失、Queue 超时、存储在轮转下有界、聚合统计以卡片呈现。验收：多日 soak 运行存在并通过。

### 0.1.0 · 正式版

Ambery Protocol 冻结，第一版 cookbook——每个写下的配方都能从文档走到桌面，首启体验配得上安装包。

## 0.1.0 之后

正式版之后的方向登记在 [post-0.1.0.zh.md](post-0.1.0.zh.md)，每项能力一段简述。
