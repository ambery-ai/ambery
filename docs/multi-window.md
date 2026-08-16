# Multi-Window Design

> The window scheme is multi-window (docs/tauri-shell.md §Window Form): pet / chat / menu / shelf static small windows + one dynamic `card-<id>` window per Card, all as independent OS windows.

## Window Breakdown

| label | Size | Purpose | When Visible |
|---|---|---|---|
| `pet` | 116×40 (initial seed; recomputed at runtime by the docs/pet-window-size.md formula) | pet + kaomoji + drag | Always |
| `chat` | 320×380 | Chat panel | When invoked (right-click toggle: chat:toggle) |
| `menu` | 380×560 | Settings panel (schema-driven, docs/config.md) | Pops up on tray right-click, hides on focus loss |
| `shelf` | pet×3 (clamped to 180–480×120–240) | Cards Shelf (transient popup layer for Card management, not a Surface, docs/view.md) | pet middle-click: pops up to the upper right over pet; closes on middle-click / focus loss |
| `card-<id>` | Dynamic (measured from content; offsetWidth/Height already include border) | Single Card (Component) | When the Card is alive and visible |

Static windows (pet/chat/menu) are all small windows with `transparent: true` + `decorations: false` + `alwaysOnTop: true` (no full-screen transparent layer spread across the middle), so there is no problem of blocking desktop clicks. `card-<id>` is created by pet via `ensure_card_window` (authoritative Rust registry decision, docs/case-runner.md §Window Decision Hoisted) — one independent window per id, updated in place for the same id (continuous management protocol, docs/components.md).

Transparent-window chrome rules (the single source for styles is the comment at the top of `styles.css`): filled panels (chat/shelf/menu) use `box-sizing: border-box` + 100% to fill the window, and the border is drawn inside the box itself, so it naturally falls within the window bounds; Card measurement includes the border, so the window exactly wraps the content; pet is compensated by the window-size formula + BORDER_PX (docs/pet-window-size.md).

## Data Channels

Each window connects to ambery-core independently (Tauri IPC; effect events are the backend downlink bus):

| Window | Subscribes | Sends |
|---|---|---|
| `pet` | `effect` (render_component / close_component / set_autonomy / config / top_state), `shelf:visibility` / `shelf:dismiss`, `engine:place` / `engine:moved` / `engine:remove` / `engine:release` | `list_cards`, `update_card_layout` (Card persistence, docs/components.md §Card File), `ensure_card_window` / `close_card_window` (window decisions, docs/case-runner.md §Window Decision Hoisted), `shelf:toggle` (middle-click) |
| `card-<id>` | `card:spec` (forwarded by pet), `cards:hide` / `cards:show` | `pushEvent`, `engine:moved` (drag write-back), `close_card_window` (× / OS close funnel) |
| `chat` | `chat:toggle` / `chat:hide` / `chat:show`, `effect` (context_changed) | `appendUserMessage` |
| `menu` | `effect` (config) | `get_config_schema` / `set_config` / `toggle_pet` / `quit_app` / `export_theme` / `import_theme` |
| `shelf` | `shelf:toggle` / `shelf:hide` | `set_card_user_closed`, `pushEvent` (dismiss), `shelf:visibility` / `shelf:dismiss` (sent to pet) |

Each window subscribes only to the messages it cares about, independently of the others.

## Position Sync and Card Placement

Positioning semantics follow docs/window-follow.md: pet owns the positioning engine (pet-relative coordinate system); chat/cards request positions via `engine:place`, and after dragging ends, offsets are written back via `engine:moved`; when pet is dragged or restored from the tray, the engine computes each window's position on the spot and broadcasts it (`chat:show` / `cards:show` carry coordinates). A Card's manual offset is persisted across restarts in `.card.json` (`seedManual` takes over at startup).

## Window Creation and Lifecycle

- Static windows are all defined with `visible: false` in `tauri.conf.json`; they are created and pinned in Rust `setup`.
- The pet window immediately `show()`s; chat is shown/hidden by frontend events; menu pops up on tray right-click and auto-hides on focus loss.
- `card-<id>` lifecycle: creation/reuse is decided by the authoritative Rust `ensure_card_window` registry (Card windows do not subscribe to the global render stream; they receive only the targeted `card:spec` event); close action, user ×, and shelf dismiss are all funneled into `close_card_window` (destroy + delete `.card.json`); at startup, pet pulls live Cards via `list_cards` and rebuilds visible windows (docs/components.md §Card File).
- pet's CloseRequested → hide to tray; pet visibility is controlled by the settings-panel button (`toggle_pet`), and chat/cards are hidden together with it.

## Window Z-Order

Fixed relative order within the TOPMOST layer; zero interaction with external windows' z-order.

| Depth | Window |
|---|---|
| bottom | `pet` |
| ↑ | `shelf` |
| ↑ | `chat` |
| ↑ | `menu` |
| top | `card-<id>` |

Contract:
- Internal windows never fight each other for the layer top: the internal order is the only invariant.
- External z-order is untouched: the coordinator only re-raises this app's own windows, preserving the original fight-back intent (defending against external apps stealing topmost) without changing any external window.

Windows implementation (`window.rs`, `cfg(windows)` gated):
- One coordinator thread replaces the per-window fight-back threads: it holds the ordered HWND table of all internal windows and every 500ms re-raises them bottom-to-top with `SetWindowPos(HWND_TOPMOST, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE)`.
- TOPMOST-layer order is "last raiser wins", so after each pass the layer order equals the table order exactly.
- `card-<id>` joins the table at `ensure_card_window` and leaves at `close_card_window` (`Mutex<Vec<HWND>>`; the registry pattern mirrors `CardWindowRegistry`).
- `SWP_NOACTIVATE` preserves focus semantics (menu's hide-on-focus-loss stays intact).

Non-Windows: `alwaysOnTop` is declared in tauri.conf.json (Tauri cross-platform handling); no coordinator thread.

## Window Form

- Window count: 4 static (pet/chat/menu/shelf) + N dynamic Card windows.
- Small windows stay out of the way; no click-through is needed.
- Dragging and positioning: the frontend computes coordinates → IPC `window.setPosition()`.
- Card/Chat positioning: independent-window `setPosition()`.
- WebView2 instances: 4+N (~50MB each).
