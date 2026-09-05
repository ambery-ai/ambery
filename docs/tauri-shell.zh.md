# Tauri Shell 设计

[English](tauri-shell.md) | 中文

> 概念定义见 concepts.md §3（View 物理容器）。本文档定壳形态。
>
> 窗口方案为多窗口（`docs/multi-window.md`）；全屏 `maximized: true` 方案因 WebView2 点击穿透（`WS_EX_TRANSPARENT`）不稳定不采用。

## 形态：静态小窗口 + 动态卡片窗

pet / chat / menu / shelf 四个静态窗口 + 每卡一个动态 `card-<id>` 窗，均为 `transparent: true` + `decorations: false` 的独立 OS 窗口（tauri.conf.json）：
- `transparent: true` + `decorations: false` + `shadow: false`
- `alwaysOnTop: true` + 单一 500ms 自底向上 TOPMOST 重申协调器（Windows；`cfg(windows)` 门控；顺序契约见 docs/multi-window.md §窗口 Z-Order）
- `focus: false`：启动不抢焦点（shelf 例外：focus true，失焦即关语义）
- `skipTaskbar: true`；静态窗口 `visible: false`（pet setup 后即 show；chat/menu/shelf 事件驱动显隐）
- `winvd::pin_window`：跨虚拟桌面 pin（Windows）
- 窗口间通过 Tauri IPC 事件同步位置（`pet:moved`）；card 窗创建/关闭由 Rust `ensure_card_window` / `close_card_window` 权威决策（docs/case-runner.md §窗口决策上提）

pet 初始种子 116×40（运行时被 pet-window-size.md 公式重算），chat 320×380，menu 380×560，shelf 打开时按 pet 物理尺寸 ×3 现算。窗口 url 以 hash 区分：`index.html`（pet）、`index.html#menu`、`index.html#chat`、`index.html#shelf`、`index.html#card`。

## 前端适配

- 每个窗口加载 `index.html`，`main.ts` 按窗口 label 路由到 `pet.ts` / `menu.ts` / `chat-window.ts` / `shelf.ts` / `card-window.ts`
- 各窗口独立连接 ambery-core（Tauri IPC；浏览器调试走 RemoteBridge HTTP+WS），读取经前端 store 收敛（docs/case-runner.md §前端读取架构）
- pet 拖拽走 IPC `window.setPosition()`，同时 emit `"pet:moved"` 事件
- chat/cards 窗口经 positioning engine 请求位置（pet 持有 engine，`engine:place` / `engine:moved` 协议）

## 内嵌 core（单进程架构决定）

前端与 core 通信走 Tauri 原生 IPC（`#[tauri::command]` + `invoke()` + `app_handle.emit()`）。仅外部 hook 脚本走 HTTP `POST /hook`（进程外不可用 Tauri command），薄 server 绑 127.0.0.1:47600 仅此用途。

## 跨平台与 UIA 边界

所有平台的默认运行方式是 **Hook 驱动**：Hook 是跨平台核心输入，pet、Chat、配置、卡片与核心处理流程都不得依赖 UIA。Windows UIA 只是一项由用户明确启用的可选增强，不是默认读通道，也不能成为 Hook 的前置条件。

```text
所有平台
  默认：Hook 驱动的核心体验

Windows
  可选：用户启用 UIA → 使用 Windows UIA sidecar 增强读取能力

macOS / Linux
  只提供：Hook 驱动的核心体验
  不提供：UIA 开关、UIA sidecar、Windows UIA 调用路径
```

因此，非 Windows 构建不是“找不到 sidecar 后的降级版”：UIA sidecar 不编译、不打包，Windows 专属实现也不参与其编译或链接。Windows 目标则一律编译 UIA 相关代码，并携带已编译的 UIA sidecar；“可选”只表示运行时默认不启动、不使用，用户选择启用后才走该能力路径。

当前隔离状态：

- Tauri shell 的 Windows 专属依赖（`winvd` / `windows`）收进 `[target.'cfg(windows)'.dependencies]`；`window.rs` 的 pin/fight-back 与 `menu_window.rs` 的 `SetForegroundWindow` 由 `#[cfg(windows)]` 门控，非 Windows 目标为最小替代（tauri.conf.json 的 `alwaysOnTop` + `set_focus`）。
- core 的 UIA sidecar 发现（`paths::sidecar_exe`）在非 Windows 目标恒为 `None`——不发现、不启动、不使用；sidecar 客户端是纯 std 进程通信代码，非 Windows 目标上无调用路径（Option 链天然降级，`sidecar_enabled=false`）。C# sidecar 目标为 `net9.0-windows` 且发布形态为 self-contained win-x64，不进入非 Windows 打包（docs/terminal/wt/sidecar.md §打包）。
- 残余验证边界：非 Windows 目标的 `cargo check --target` 需要交叉工具链（`ring` 经 reqwest 引入原生 C 构建），本机不可行；`cfg(not(windows))` 分支为最小 stub，正确性由评审保证，交叉编译验证待 CI。

## 全局唤起快捷键

**0.1.0 明确 cut**（docs/post-0.1.0.md）：不实现全局快捷键；托盘/手势是当前唯一唤起路径。

## 模块拆分

`src-tauri/src/`：

| 文件 | 职责 |
|---|---|
| `main.rs` | 薄组装层：三窗口创建 + pin、托盘、core 启动、IPC 命令（含 `ensure_card_window` / `close_card_window` 窗口决策，docs/case-runner.md §窗口决策上提） |
| `window.rs` | 窗口 pin（winvd）+ z-order 协调器（自底向上 TOPMOST 重申，docs/multi-window.md §窗口 Z-Order）——`cfg(windows)` 门控 |
| `tray.rs` | 系统托盘（显示/隐藏/退出）+ CloseRequested 隐藏到托盘 |
| `menu_window.rs` | 设置面板弹出/失焦隐藏；前台聚焦 Windows 走 Win32（`cfg(windows)` 门控） |
| `tauri_runtime_actions.rs` | Rust 壳侧运行时动作层（toggle_pet 等的逐动作 effect 记录，docs/effect-reporting.md） |

