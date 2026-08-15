# 多窗口方案设计

> 窗口方案为多窗口（docs/tauri-shell.md §形态）：pet / chat / menu / shelf 静态小窗口 + 每卡一个动态 `card-<id>` 窗，均为独立 OS 窗口。

## 窗口划分

| label | 尺寸 | 用途 | 何时可见 |
|---|---|---|---|
| `pet` | 116×40（初始种子；运行时被 docs/pet-window-size.md 公式重算） | ペット + 颜文字 + 拖拽 | 始终 |
| `chat` | 320×380 | 聊天面板 | 唤出时（右键 toggle：chat:toggle） |
| `menu` | 380×560 | 设置面板（schema 驱动，docs/config.md） | 托盘右键弹出，失焦隐藏 |
| `shelf` | pet×3（钳制 180–480×120–240） | Cards Shelf（Card 管理瞬时弹出层，非 Surface，docs/view.md） | pet 中键：遮挡 pet 向右上弹出，中键/失焦即关 |
| `card-<id>` | 动态（按内容测量，offsetWidth/Height 已含 border） | 单张卡片（Component） | 卡片存活且可见时 |

静态窗口（pet/chat/menu）都是 `transparent: true` + `decorations: false` + `alwaysOnTop: true` 的小窗口（不在中间铺全屏透明层），所以不会有挡住桌面点击的问题。`card-<id>` 由 pet 经 `ensure_card_window` 创建（Rust 权威注册表决策，docs/case-runner.md §窗口决策上提）——每 id 一个独立窗口，同 id 原地更新（持续管理协议，docs/components.md）。

透明窗口 chrome 规则（样式单源在 `styles.css` 顶部注释）：填充型面板（chat/shelf/menu）`box-sizing: border-box` + 100% 填满窗口，border 内绘于自身盒内、天然落在窗口边界内；card 测量含 border，窗口恰好包裹内容；pet 由窗口尺寸公式 +BORDER_PX 补偿（docs/pet-window-size.md）。

## 数据通道

各窗口独立连接 overseer-core（Tauri IPC；effect 事件为后端下行总线）：

| 窗口 | 订阅 | 发送 |
|---|---|---|
| `pet` | `effect`（render_component / close_component / set_autonomy / config / top_state）、`shelf:visibility` / `shelf:dismiss`、`engine:place` / `engine:moved` / `engine:remove` / `engine:release` | `list_cards`、`update_card_layout`（Card 持久化，docs/components.md §Card 文件）、`ensure_card_window` / `close_card_window`（窗口决策，docs/case-runner.md §窗口决策上提）、`shelf:toggle`（中键） |
| `card-<id>` | `card:spec`（pet 转发）、`cards:hide` / `cards:show` | `pushEvent`、`engine:moved`（拖拽回写）、`close_card_window`（× / OS 关闭收口） |
| `chat` | `chat:toggle` / `chat:hide` / `chat:show`、`effect`（context_changed） | `appendUserMessage` |
| `menu` | `effect`（config） | `get_config_schema` / `set_config` / `toggle_pet` / `quit_app` / `export_theme` / `import_theme` |
| `shelf` | `shelf:toggle` / `shelf:hide` | `set_card_user_closed`、`pushEvent`（dismiss）、`shelf:visibility` / `shelf:dismiss`（发 pet） |

每个窗口只订阅自己关心的消息，彼此独立。

## 位置同步与卡片定位

定位语义以 docs/window-follow.md 为准：pet 持有 positioning engine（pet 相对坐标系），chat/cards 经 `engine:place` 请求位置、拖拽结束经 `engine:moved` 回写偏移；pet 拖拽/托盘恢复时 engine 现算各窗口位置并广播（`chat:show` / `cards:show` 携带坐标）。Card 的手动偏移跨重启持久化在 `.card.json`（启动时 `seedManual` 接棒）。

## 窗口创建与生命周期

- 静态窗口在 `tauri.conf.json` 中都定义为 `visible: false`；Rust `setup` 中创建并 pin
- pet 窗口立即 `show()`；chat 由前端事件驱动显隐；menu 托盘右键弹出、失焦自动隐藏
- `card-<id>` 生命周期：创建/复用由 Rust `ensure_card_window` 权威注册表决策（card 窗不订阅全局 render 流、只收 `card:spec` 定向事件）；close action、用户 × 与 shelf dismiss 统一收口 `close_card_window`（destroy + 删 `.card.json`）；启动时 pet 经 `list_cards` 拉取存活卡片重建可见窗口（docs/components.md §Card 文件）
- pet 的 CloseRequested → hide 到托盘；pet 显隐由设置面板按钮（`toggle_pet`）控制，连带 chat/cards 系统隐藏

## 窗口形态

- 窗口数：4 静态（pet/chat/menu/shelf）+ N 动态 card 窗
- 小窗口不挡路，不需要点击穿透
- 拖拽与定位：前端计算坐标 → IPC `window.setPosition()`
- 卡片/聊天定位：独立窗口 `setPosition()`
- WebView2 实例：4+N（每个 ~50MB）
