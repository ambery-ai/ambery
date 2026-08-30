# Spec — packages/apps

[English](spec.md) | 中文

## 技术选型

| 形态 | 技术 |
|---|---|
| tauri | Tauri 2 壳 + vanilla TypeScript 前端 |
| webui | 纯 web 形态，同一前端代码的第二宿主 |

取舍：

- **一份前端，两个宿主**——窗口管理与 IPC 因形态而异；前端代码不变。bridge 层（bridge.ts / effects）是缝；宿主实现它，UI 保持宿主无关。
- **vanilla TS，不用框架**——展示逻辑简单，框架的抽象成本大于收益。浏览器调试模式直接跑 vite；UI 可用 Chrome DevTools 测试。

## 架构决定

1. **每个形态是同一前端核心之上的一个包**：tauri 与 webui 内嵌/服务同一份 src；差异在宿主层（窗口管理、IPC 传输、打包）。
2. **形态只经既定通道与 core 通信**——打包态 tauri 走原生 IPC；浏览器/webui 态走薄 HTTP+WS loopback（127.0.0.1）。两种模式同一份前端代码。

## 固定约束

- 不用 UI 框架；vanilla TS。
- UI 交互禁止浏览器原生弹窗（alert / prompt / confirm）：错误与输入用应用内 UI 元素表达。
