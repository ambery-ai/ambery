# 终端与 Agent CLI 接入面全景调查

> Date: 2026-08-31

接入协议（docs/access-protocol.md）的地形图。ambery 现状模型：**读通道** = `TerminalAdapter`（`enumerate() -> Option<Vec<TabInfo>>` / `read(tab) -> ReadOutcome{Content|Gone|Error}`，实现：WT 走 C# UIA sidecar stdio JSONL、zellij 走 CLI、内存 mock）；**推送通道** = 装进 agent CLI 的 hook 脚本 → POST 内嵌 HTTP（session_start/user_prompt_submit/stop/session_end/notification），并经 Claude Code SessionStart hook 的 `sessionTitle` 输出写 tab 定位标记 `<project>·<sid8>`。

## 终端对比

| 对象 | 接口形态 | 可得数据 | 接入成本 | ambery 角色映射与缺口 |
|---|---|---|---|---|
| Windows Terminal | `wt` CLI 仅启动/聚焦（new-tab/split-pane/focus-tab/-w 窗口寻址），**无查询/读取 API**；OSC 标题生效（应用标题即 tab 标题） | 官方面：仅写入式命令。枚举/读屏靠 UIA 绕行 | 高（需 UIA sidecar）——**已建成** | **读通道**（已接入）。gap：官方无任何事件面，tab 关闭等只能轮询对账 |
| zellij | CLI actions（`list-panes -a --json`、`dump-screen [-full]`、`send-keys`/`write-chars`、`rename-tab`）；WASM 插件体系 + `zellij pipe` | 枚举 pane（id/title/cwd/command/focused/exited）✓；读屏 ✓（含 scrollback、可选 ANSI）；写 ✓；事件：插件可订阅 PaneUpdate/PaneClosed/SessionUpdate/CommandChanged/CwdChanged/PaneRenderReport（周期性吐出去 ANSI 的 pane 正文） | 小——**已建成**（CLI 轮询形态） | **读通道**（已接入）；可升级**推送通道**：常驻 WASM 插件 + pipe 直推 ambery HTTP，免轮询（未做） |
| iTerm2 | Python API（WebSocket RPC，pip 包 `iterm2`）；专有 OSC/custom control sequences | 枚举 windows/tabs/sessions ✓；读屏 ✓（`async_get_screen_contents`，`async_get_contents` 可取 scrollback）；写 ✓（`async_send_text`）；事件 ✓（ScreenStreamer 推送屏幕更新、FocusMonitor）；元数据通道：`async_set_variable`（user.* 变量） | 中：需常驻 Python sidecar（类 WT sidecar 形态）；macOS only | **读通道**（macOS 上最优接口）；session 名可 `async_set_name` 写 marker。gap：Windows 无关 |
| tmux | CLI 查询族 + `set-hook` 事件钩子 + **control mode**（`tmux -C` 常驻文本协议） | 枚举 ✓（`list-panes -a -F` 格式化输出 pane_id/title/current_command/cwd）；读屏 ✓（`capture-pane -p [-S -]` 含 scrollback）；写 ✓（`send-keys`）；事件 ✓✓：hooks（pane-exited、session-created/closed、client-attached、window-renamed…可挂任意 shell 命令）+ control mode 异步通知（%output/%window-add/%session-changed/%subscription-changed） | 小：CLI wrapper（同 zellij 形态）；control mode 常驻进程可做推送（中） | **读 + 推送**双料候选。gap：tab=window/pane 语义映射；marker 需 shell 侧 OSC 写标题 |
| ghostty | **无官方远程控制/IPC/查询接口**（config/keybind 文档全量核对）；macOS 有内置 AppleScript 词典（`macos-applescript`，默认开，官方自述支持 windows/tabs/terminals 对象查找）；keybind action `write_scrollback_file`/`write_screen_file` 仅能用户按键触发；OSC 标题生效；`notify-on-command-finish`（OSC 133 驱动，仅桌面通知） | 官方面：几乎零。AppleScript 能否读屏幕正文「未证实」；GTK/Linux 侧无任何通道 | 高：macOS 或可走 AppleScript + AX/UIA 类绕行（同 WT 路线），Linux 无门 | 当前** neither**；marker 通道（OSC 标题）可用但需配套读取面才有意义。等官方 IPC（社区呼声高，「未证实」有无 roadmap） |
| kitty | **remote control**：`kitten @ <cmd>`（可指定 unix/TCP socket，细粒度权限白名单）；kitten 插件（Python）；JSON 遥控协议文档化 | 枚举 ✓（`kitten @ ls` JSON 全树）；读屏 ✓（`kitten @ get-text`，可选 scrollback/ANSI）；写 ✓（`send-text`）；设标题 ✓（`set-tab-title`/`set-window-title`）；事件：无内建推送，需轮询或 kitten 内自定义 | 小：CLI wrapper；前提是用户开启 `allow_remote_control` + 监听 socket | **读通道**。gap：无事件推送（轮询即可，ambery 本来就是轮询模型）；Windows 无关 |
| alacritty | `alacritty msg` IPC socket：**仅** create-window / config / get-config | 枚举 ✗；读屏 ✗；写 ✗（无 send 类消息）；OSC 标题生效 | 高：读取只能走 UIA 类无障碍绕行（alacritty 的 UIA 支持度「未证实」） | **neither**（除标题 marker）。项目哲学即最小化，勿期待官方扩展面 |
| wezterm | `wezterm cli`（list --format json / get-text / send-text / set-tab-title / list-clients）；Lua 配置 API（`wezterm.mux`、`Pane:get_lines_as_text`、`get_user_vars`）；Lua 事件（`user-var-changed`、`update-status`、`bell`…）；mux server 域 | 枚举 ✓（pane/tab/window 全树 JSON：title/cwd/尺寸）；读屏 ✓；写 ✓；事件 ✓（用户配置 Lua 回调即可 HTTP 外推；OSC 1337 SetUserVar → `user-var-changed` 是现成的带内信号道） | 小：CLI wrapper；**跨平台含 Windows** | **读 + 推送**双料候选。gap：Lua 事件需用户配置注入（config 级，非纯安装） |

## Agent CLI 对比

| 对象 | 接口形态 | 可得数据（事件面） | 接入成本 | ambery 角色映射与缺口 |
|---|---|---|---|---|
| Claude Code | settings.json hooks（handler 类型：command/http/mcp_tool/prompt/agent）；MCP client（可注册 ambery 的 MCP server）；headless `-p` + SDK | 事件最全一档：SessionStart/SessionEnd/UserPromptSubmit/Stop/StopFailure/Notification/PreToolUse/PostToolUse/PreCompact/SubagentStart·Stop…；**SessionStart/UserPromptSubmit 的 hook 输出可设 `sessionTitle`**（官方字段，ambery marker 即此） | **已建成** | **推送通道**（已接入）。MCP 注册未用：给 ambery 开「agent 反问宿主」通道（读终端状态等 Tool Set 暴露给 claude） |
| Codex CLI | hooks（`hooks.json` 或 `config.toml [hooks]`，handler 类型 command/mcp_tool；信任审核流 /hooks）；`notify` 配置（turn 完成跑外部程序）；MCP client；`codex mcp-server`（自身当 MCP server）；App Server（JSON-RPC）；`codex exec` headless；Codex SDK | 事件：SessionStart/SessionEnd/UserPromptSubmit/Stop/PreToolUse/PostToolUse/PermissionRequest/PreCompact/PostCompact/SubagentStart/SubagentStop。hook stdin JSON 带 session_id/cwd/transcript_path/model；**hook 输出无 sessionTitle 类字段**（仅 continue/stopReason/systemMessage/additionalContext） | 小：与 Claude Code hook 同构，ambery-hook 模式直接平移 | **推送通道**（强候选）。gap：marker 无官方通道 → 需 shell 包装起进程前写 OSC 标题；trust review 对自动装 hook 是摩擦点 |
| opencode | **插件体系**（JS/TS，`~/.config/opencode/plugins/`，`event` 钩子订阅总线事件 + `tool.execute.before/after` 等）；`opencode serve` HTTP server（OpenAPI 3.1，`GET /session`、`GET /event` SSE、`/tui/*` 控制、`POST /mcp` 动态挂 MCP server）；MCP client；ACP；`opencode run` headless | 插件事件：session.created/updated/idle/error、permission.asked、message.updated、tui.prompt.append…（session.idle ≈ stop 语义）；server 面可直接枚举会话+SSE 推事件 | 小~中：投一个 plugin 文件即得全事件推送（config 级）；或轮询 server（端口随机，需 --port 约定） | **推送通道**（plugin 路线最优，且 packages/agents/opencode spec 正缓定 hook 形态——此调查即输入）。gap：title/marker 无官方面 |
| gemini-cli | settings.json hooks（仅 command 类型；stdin/stdout JSON）；MCP client（settings.json mcpServers）；extensions；headless；ACP mode | 事件：SessionStart/SessionEnd/BeforeAgent/AfterAgent/BeforeModel/AfterModel/BeforeToolSelection/BeforeTool/AfterTool/PreCompress/Notification；env 带 GEMINI_SESSION_ID/GEMINI_CWD；hook 输出无语义化 title 字段 | 小：hook 模式同构平移 | **推送通道**（候选）。gap：同 Codex 无 marker 通道；另注意官方公告 unpaid/Google One 档 2026-06-18 起迁往 Antigravity CLI，投入前先确认目标用户档位 |
| aider | **无 hooks、无 MCP**（options 全量核对）；`--message` 非交互单次执行；Python API（官方声明不保证兼容）；`--notifications-command`（回复就绪时跑外部命令，单一粗事件） | 事件：仅「LLM 回复就绪」一个点；无会话 id 传递契约 | 中：只能靠终端侧标题/读屏 + notifications-command 凑 | 基本 **neither**。不值得专门接；若用户在 aider 里，走终端读通道兜底即可 |
| cursor-agent | `agent` CLI：交互 TUI + print 模式（`--output-format json\|stream-json` NDJSON，含 session_id/tool_call 事件流）；`agent ls`/`resume` 枚举历史会话；MCP client（与编辑器共用 mcp.json，`agent mcp list`）；**ACP mode**（自定义客户端协议驱动 CLI）；Cloud 交接；cli-config `display.showStatusIndicators` 会写终端标题状态 | Cursor hooks（hooks.json：sessionStart/sessionEnd/beforeSubmitPrompt/preToolUse/stop/afterAgentResponse…stdio JSON）官方文档面向 IDE 与 cloud agents；**cursor-agent CLI 是否触发同一套 hooks「未证实」** | 中：hooks 若可用则小；否则只有 headless 流式输出（只读、非交互场景） | **推送通道候选（待证实）**；ACP 是另一条重路线（ambery 当 client 驱动 agent，超出本报告的读/推送模型）。gap：交互 TUI 下的事件面不明 |

## Agent CLI 专项注记

- **title/marker 通道**：只有 Claude Code 有官方 `sessionTitle` hook 输出（已利用）。Codex/gemini/opencode/cursor-agent 的 hook/插件均可拿到 session_id，但**写终端标题需旁路**：最通用做法是包装入口命令，启动前 `printf '\e]2;<project>·<sid8>\a'`（OSC 2，各终端普遍支持，WT 文档确认应用标题即 tab 标题）。
- **MCP 注册方向**：五家（Claude Code/Codex/opencode/gemini/cursor-agent）都是 MCP client，ambery 可注册自身 MCP server 进去 → 给 ambery 开第三条通道（agent 主动调宿主 Tool Set，如自查终端状态），与读/推送两通道正交。aider 无 MCP。
- **事件语义映射**：各家的 stop 等价物 —— Claude `Stop`、Codex `Stop`、opencode `session.idle`、gemini `AfterAgent`、cursor `stop`；session 起点各家均有 SessionStart 类。事件 schema 都带 session id + cwd，ambery 现有 `{event, session_id, cwd, kind}` 载荷契约可直接覆盖。

## 接入优先级建议

1. **Codex CLI（推送通道）**：hook 体系与 Claude Code 几乎同构（settings 文件、stdin JSON、matcher），ambery-hook 模式平移成本最低；事件覆盖 ambery 五元组绰绰有余。唯一新增工作是 marker 旁路（OSC 标题包装）。**推荐下一个做。**
2. **opencode（推送通道，plugin 路线）**：packages/agents/opencode/spec 已缓定 hook 形态等真实行为——本调查给出答案：投 `~/.config/opencode/plugins/ambery.js`，订阅 `session.created/idle/updated` + `tui.prompt.append` 直接 POST 到宿主，比 CLI 轮询和 server 模式都干净（plugin 随 opencode 生命周期自动起，无需约定端口）。
3. **wezterm / tmux（读通道，二选一按平台）**：两者 CLI 都是 zellij adapter 的直接类比（`wezterm cli list/get-text`、`tmux list-panes/capture-pane`），接进 `TerminalAdapter` 是小工作量。要 Windows 覆盖选 wezterm；要 SSH/远端会话与事件推送选 tmux（`set-hook`/control mode 还能兼推送通道）。kitty 同质但无 Windows，列候补。

明确不做：ghostty（无官方面，绕行 ROI 低）、alacritty（同）、aider（无 hook/MCP，终端读通道兜底即可）。

## 来源

- WT: learn.microsoft.com/windows/terminal/command-line-arguments（ms.date 2025-11）；zellij: zellij.dev/documentation/cli-actions.html、plugin-api-events.html
- iTerm2: iterm2.com/python-api/、/python-api/session.html；tmux: man7.org/linux/man-pages/man1/tmux.1.html（HOOKS / CONTROL MODE / FORMATS 节）
- kitty: sw.kovidgoyal.net/kitty/remote-control/；wezterm: wezterm.org/cli/general.html；alacritty: github.com/alacritty/alacritty extra/man/alacritty-msg.1.scd；ghostty: ghostty.org/docs/config/reference、/docs/config/keybind/reference
- Claude Code: code.claude.com/docs/en/hooks；Codex: developers.openai.com/codex/hooks、/codex/notifications；opencode: opencode.ai/docs/plugins、/docs/server；gemini-cli: geminicli.com/docs/hooks/；aider: aider.chat/docs/scripting.html、/docs/config/options.html；cursor-agent: cursor.com/docs/cli/overview、/docs/cli/mcp、/docs/cli/reference/output-format、cursor.com/docs/hooks

「未证实」清单：ghostty AppleScript 能否读屏幕正文；ghostty 官方 IPC roadmap 有无；alacritty 的 UIA 支持度；cursor-agent CLI 是否触发 Cursor hooks.json。
