# WtAdapter 进程协议

> terminal-adapter 的一个实现（docs/terminal-adapter.md §实现）。本文档定 WtAdapter 独立进程的 stdio JSONL 协议、命令集与生命周期。技术选型见 spec.md（UIA 保留 C#，Rust 调用）。

## 进程模型

- sidecar 是独立 console exe（.NET 9，`UseWPF` 引入 UIA 程序集），Tauri sidecar 模式随包分发；debug 阶段由 case-runner 经 `AMBERY_SIDECAR=<exe路径>` 启动。
- **协议：stdio JSON Lines**——stdin 每行一个 JSON 请求，stdout 每行一个 JSON 响应。Rust `SidecarClient` 持进程句柄，Mutex 串行化请求（UIA 操作本身不可并行——切 Tab 是全局状态）。
- 崩溃处理：每次请求检查进程存活，退出则重启一次再重试；仍失败返回 None（读通道降级回 Context，AmberyBackend 语义不变）。

## 打包（已定案）

- **self-contained win-x64，非单文件**：`sidecar.csproj` 固化 `RuntimeIdentifier=win-x64` / `SelfContained=true` / `PublishSingleFile=false`；用户机器无需 .NET 9 Desktop Runtime。
- 发布命令：`dotnet publish sidecar/sidecar.csproj -c Release` → `sidecar/bin/Release/net9.0-windows/win-x64/publish/ambery-uia-sidecar.exe`。
- Tauri 侧：`bundle.active` 当前为 false（发布轮开启）。开启 Windows 打包时在 `app/src-tauri/tauri.conf.json` 的 `bundle.externalBin` 加入 `../../sidecar/bin/Release/net9.0-windows/win-x64/publish/ambery-uia-sidecar.exe`；不要在非 Windows 构建常驻该配置——Tauri build script 会按当前平台解析 externalBin 路径。
- 路径发现优先级（`core/src/paths.rs`）：`AMBERY_SIDECAR` env > 当前 exe 旁 > 当前 exe 旁 `sidecar/` > Release publish > Debug。Windows 真机验证前，publish 布局未经打包流水线实测（标 `dev/issues.md`）。

## 命令集

```
→ {"cmd":"list_windows"}
← {"ok":true,"windows":[{"hwnd":12345,"title":"..."}]}

→ {"cmd":"list_tabs","hwnd":12345}
← {"ok":true,"tabs":[{"index":0,"name":"✳ demo-webapp","selected":true}]}

→ {"cmd":"find_tab","name":"demo-webapp"}        # 子串匹配，跨所有窗口
← {"ok":true,"hwnd":12345,"index":2,"name":"✳ demo-webapp"}

→ {"cmd":"read_tab","hwnd":12345,"index":2}          # SelectionItemPattern.Select() 切换（~200ms）+ 读全文
← {"ok":true,"text":"..."}

→ {"cmd":"read_active_tab","hwnd":12345}             # 不切换，非侵入只读
← {"ok":true,"text":"..."}

→ {"cmd":"count_processes","name":"claude"}         # 进程计数（启动扫描 N/M/K 三方对账用）
← {"ok":true,"count":13}

错误统一：{"ok":false,"error":"..."}
```

- `read_tab` 会真实切换用户的标签页（概念 §7：200ms 成本）——Timer 兜底扫描/fetch_terminal 的语义就是「切过去读」；`read_active_tab` 留给非侵入场景（调试、当前窗口快读）。
- 响应 text 是 UIA 网格原文（右填充、含 spinner），Filter 在 Rust 侧处理（docs/filter.md）——sidecar 不过滤，职责单一。

## 读通道接线（Terminal Adapter，docs/terminal-adapter.md）

```
fetch_terminal / Timer scan（instance 名 = Tab 名）
  → find_tab(instance) → 找不到 → None（回退 Context 最新记录）
  → read_tab(hwnd, index) → text
```

instance ↔ Tab 的 1:1 关系见 concepts §9。**Hook→Tab 定位已解决**：sessionTitle marker（`<project>·<sid8>` 前缀不变量，docs/hook.md §marker 定位），find_tab 按 marker 精确命中；定位结果缓存进注册表（惰性重试，找到即冻结）。

## 视野模型

**可选便利（不强制）**：Windows Terminal 开启「在所有桌面上显示此应用的窗口」后，全部虚拟桌面的 WT 窗口/tab/内容对 UIA 全量可见可读（实测：17/17 窗口 uncloak，读空的窗口恢复 6 tab + 5415 字符全量）。未开启也照常工作——其他 VD 的实例 hook 照收，读通道回退 Context；需要全量时 agent 可用 `fetch_terminal`（`vd_switch: true` 显式同意）切过去读（docs/hook.md §VD 切换能力）。

机制背景：其他 VD 的窗口被 DWM 打 `cloaked=2`（壳层隐身——挂起视觉树，进程/消息循环不死），EnumWindows 能见句柄+标题但 **UIA 树不实体化（读空）**；「全桌面显示」设置解除 cloaking，VD 切换使其临时解除。

**真正的视野边界**（设置也救不了）：

- **非 Windows Terminal 终端**（WezTerm / VS Code 终端 / ConEmu）：CASCADIA 类名不匹配，不可见
- **cloaked 窗口的背景 tab**（未开启设置时）：窗口标题只反映活动 tab，背景 tab 不可见

VD 切换不作为后台路径，但作为 **agent 显式能力开放**（开放原则）。

## 常驻与拉起（简化语义）

app 启动自动发现 exe 并启用（`AMBERY_SIDECAR` env > 仓库约定位置），进程惰性拉起（首次请求 spawn）。**死了即弃，下次请求现拉起**（冷启实测 ~200ms）——无保活预检、无心跳；每次请求最多两次尝试，仍失败返回 None（读通道降级回 Context，AmberyBackend 语义不变）。

## 阻塞边界的已落方案

`TerminalAdapter` 的 locate/read 是同步 trait 调用；AmberyBackend 的读入口（`read_terminal`）把它们整体放 `tokio::task::spawn_blocking`——sidecar 的进程 IO / Mutex 等待 / 5s 切换限流 sleep 都在 blocking 线程池内，不占用 tokio worker。SidecarClient 自身保持同步协议客户端，不引入嵌套 runtime。
