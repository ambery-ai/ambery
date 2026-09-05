# 开发

[English](DEVELOPING.md) | 中文

开发 ambery 时如何构建、运行、观测与调试。除注明外均在仓库根目录执行。

## 构建与测试

```bash
cargo test --workspace                          # Rust workspace（core + case runner + activity 查看器）
cargo run -p ambery-case -- frontend --silent   # 前端 headless case（mock/keyless，内嵌 core）
cd app && npm ci && npm run build               # 前端：tsc + vite build
```

## 跑通全栈（浏览器 debug UI）

三个进程，三个终端：

```bash
# 1) 本地 OpenAI 兼容 LLM 替换（默认端口 47777）
python3 scripts/debug_brain.py

# 2) core 后端（HTTP + WS 于 127.0.0.1:47600；AMBERY_PORT 覆盖端口）
cargo run -p ambery-case -- serve --brain-addr http://127.0.0.1:47777

# 3) 前端 dev server —— 打开打印的 URL（通常 http://localhost:5173）
cd app && npm run dev
```

debug brain 是最小阈值决策源，不是对话模型：满足其通知规则的 hook 产出通知卡，普通聊天得到空回复。要真实对话，把 `--brain-addr` 指向任意 OpenAI 兼容端点。

## 实时观察 storage

```bash
cargo run -p ambery-core --bin ambery-activity -- \
  --dir ~/.config/ambery/storage --trajectory --follow
```

这是标准开发命令：轨迹账本（session / turn / event；docs/tools.md）随 `--follow` tail 新写入的记录。storage 位于配置根（默认 `~/.config/ambery/storage`，`AMBERY_STORAGE_DIR` 可覆盖）；浏览快照时显式传 `--dir`。

## 模拟 hook

真实产品由 Claude Code hooks 驱动；开发期可经 HTTP 驱动同一条路径：

```bash
curl -X POST http://127.0.0.1:47600/hook -H 'Content-Type: application/json' \
  -d '{"event":"session_start","session_id":"dev-1","cwd":"/tmp","kind":"claude"}'
curl -X POST http://127.0.0.1:47600/hook -H 'Content-Type: application/json' \
  -d '{"event":"user_prompt","session_id":"dev-1","cwd":"/tmp","kind":"claude","prompt":"hello"}'
```

`kind` 必须是过滤器名（`claude` / `opencode`）。经 `GET /state`、`GET /context`、WS 流与 storage 文件（docs/storage.md）观察 effect。

## Tauri 壳

```bash
cd app && npx tauri build               # 唯一正确的构建入口（见下）
cd app/src-tauri && AMBERY_PORT=47601 ./target/release/ambery   # 非默认端口运行
```

壳内嵌已构建的前端与 core。打包（`.app` bundle）在发布轮前不启用（docs/terminal/wt/sidecar.md）。

**壳构建必须走 `npx tauri build`，绝不要裸 `cargo build`。** 前端 `dist/` 由 build script 嵌入 `tauri-codegen-assets/`，而 build script 不监听 `dist/`；tauri CLI 通过注入 `TAURI_CONFIG`（`cargo:rerun-if-env-changed=TAURI_CONFIG`）强制 build script 重跑。裸 `cargo build --release` 可能复用过期的 build script 输出，嵌入旧版或空前端——壳呈现为空白 pet/UI。`npx tauri build` 先跑 `npm run build`（tsc + vite），因此总是嵌入当前前端。

开发期热更新迭代：把 vite dev server 固定到 `tauri.conf.json` 的 `devUrl` 端口（127.0.0.1:5174）——`cd app && npm run dev -- --port 5174 --strictPort`——再在 `app/` 下跑 `AMBERY_PORT=47602 npx tauri dev`（壳内嵌自己的 core server，`AMBERY_PORT` 必须避开独立后端）。

**macOS：`./target/release/ambery` 后窗口空白/缺失。** 两种不同根因，症状相同（屏幕上无窗口；用 `swift -e 'import CoreGraphics; CGWindowListCopyWindowInfo(...)'` 确认）：

1. **嵌入过期前端**——裸 `cargo build --release`（非 `npx tauri build`）可能嵌入旧版或空 `dist/`，壳呈现空白 UI。修复：走 `npx tauri build` 构建。
2. **WebKit 缓存目录缺失**——`~/Library/Caches/ambery/WebKit/ServiceWorkers/` 不存在时 WebKit 初始化失败，webview 永不加载（日志无 `[tauri-cmd]` 行），窗口存在但不可见。修复：`mkdir -p ~/Library/Caches/ambery/WebKit/ServiceWorkers` 后重启。日志症状：`could not create directory ".../WebKit/ServiceWorkers" ... Operation not permitted`。

## 查看窗口

运行时调用 locate 列出所有 ambery 窗口的位置/尺寸/可见性，运行后在桌面上停留 10 秒边框（红色=可见，青绿=隐藏）：

Windows PowerShell：
```bash
pwsh -NoProfile -File tools/locate.ps1 -Highlight
```

macOS：
```bash
swift tools/locate.swift --highlight
```

## 调试 LLM 失败

把后端指向死端点以演练非静默失败路径：

```bash
AMBERY_PORT=47630 cargo run -p ambery-case -- serve --brain-addr http://127.0.0.1:9
```

该轮回退 debug agent，UI 收到 `llm_error` 帧而非静默失败。
