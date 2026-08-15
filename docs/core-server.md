# Core Server

Tauri 进程内薄 HTTP server，默认绑 `127.0.0.1:47600`，只承载外部 hook 收口。

## 端口语义

- 默认 `47600` 是 hook 脚本的投递契约；`AMBERY_PORT` 可显式覆盖（换端口必须同步更新 hook 配置）。
- **不做随机回退**：端口被占用时打印可读错误并退出——随机换端口会让外部 hook 静默投递失败，比启动失败更难诊断。

## 职责

- **仅**：外部 hook 脚本 POST `/hook` 接入（PowerShell，进程外。Tauri command 无法跨进程调）

## 不在 scope

以下职责由 Tauri 原生能力承载：

| 职责 | 承载 |
|--------|--------|
| 前端 HTTP API（state/context/config/events） | `#[tauri::command]` + `invoke()` |
| effects 广播 + WS 推送 | `app_handle.emit()` + 前端 `listen()` |
| timer 后台任务 | Tauri async runtime `spawn`（原已在 runtime 内） |

## 路由

| 方法 | 路径 | 用途 |
|------|------|------|
| POST | `/hook` | 外部 hook 脚本触发（fire-and-forget，唯一保留的 HTTP 端口） |

## 前端通信

Tauri 原生 IPC（不经过 47600）：

- **前端 → Rust**：`invoke("get_state")` / `invoke("append_user", {text})` 等 Tauri command
- **Rust → 前端**：`app_handle.emit("effect:render_component", spec)` → 前端 `listen()` 接收

## debug 模式完整 router

case-runner 以完整 `router()` 启动（`ambery-case serve`，浏览器调试 RemoteBridge 消费，docs/case-runner.md §CLI）：`/state` `/context` `/queue/user` `/events` `/config` `/config/schema` `/effect` `/ws`，另有：

- `GET /cards`、`POST /cards/layout`、`POST /cards/user_closed`——与 Tauri command `list_cards` / `update_card_layout` / `set_card_user_closed` 同一 core 逻辑（双运输层共享），供 RemoteBridge 消费（TS 子进程 / 浏览器调试）
- `POST /debug/effect`（mock feature）——向 effect 下行总线注入任意 effect 消息，headless 测试确定性驱动 render/close/config 事件，不经 LLM
- 端口默认 47600，`AMBERY_PORT` 可覆盖（沙盒用独立端口避让生产）；storage/config 目录经 `AMBERY_STORAGE_DIR` / `AMBERY_CONFIG_DIR` 隔离

## 相关文档

- `agent-loop.md` §协议：Tauri IPC + `/hook` 链路
- `hook.md`：外部 hook 脚本接入
- `timer.md`：timer 后台任务逻辑
