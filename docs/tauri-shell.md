# Tauri Shell Design

> See concepts.md §3 (View physical container) for the concept definition. This document defines the shell form.
>
> The window approach is multi-window (`docs/multi-window.md`); the fullscreen `maximized: true` approach is not adopted because WebView2 click-through (`WS_EX_TRANSPARENT`) is unstable.

## Form: static small windows + dynamic Card windows

Four static windows — pet / chat / menu / shelf — plus one dynamic `card-<id>` window per Card, all independent OS windows with `transparent: true` + `decorations: false` (tauri.conf.json):
- `transparent: true` + `decorations: false` + `shadow: false`
- `alwaysOnTop: true` + single 500ms bottom-to-top TOPMOST re-raise coordinator (Windows; gated by `cfg(windows)`; order contract in docs/multi-window.md §Window Z-Order)
- `focus: false`: does not steal focus at startup (shelf is the exception: focus true, close-on-blur semantics)
- `skipTaskbar: true`; static windows `visible: false` (pet shows right after setup; chat/menu/shelf visibility is event-driven)
- `winvd::pin_window`: pin across virtual desktops (Windows)
- Window positions are synchronized between windows via Tauri IPC events (`pet:moved`); Card window creation/closing is authoritatively decided by Rust `ensure_card_window` / `close_card_window` (docs/case-runner.md §window decision hoisting)

pet initial seed 116×40 (recomputed at runtime by the pet-window-size.md formula), chat 320×380, menu 380×560, shelf computed on open as pet physical size ×3. Window urls are distinguished by hash: `index.html` (pet), `index.html#menu`, `index.html#chat`, `index.html#shelf`, `index.html#card`.

## Frontend adaptation

- Each window loads `index.html`; `main.ts` routes by window label to `pet.ts` / `menu.ts` / `chat-window.ts` / `shelf.ts` / `card-window.ts`
- Each window connects to ambery-core independently (Tauri IPC; browser debugging uses RemoteBridge HTTP+WS); reads converge through the frontend store (docs/case-runner.md §frontend read architecture)
- pet dragging uses IPC `window.setPosition()` and emits the `"pet:moved"` event
- chat/Card windows request positions through the positioning engine (pet holds the engine; `engine:place` / `engine:moved` protocol)

## Embedded core (spec.md architecture decision)

Frontend-core communication uses Tauri native IPC (`#[tauri::command]` + `invoke()` + `app_handle.emit()`). Only external hook scripts use HTTP `POST /hook` (Tauri commands are unavailable out-of-process); a thin server bound to 127.0.0.1:47600 serves only this purpose.

## Cross-platform and UIA boundary

The default runtime mode on all platforms is **hook-driven**: hook is the cross-platform primary input, and pet, Chat, configuration, Cards, and the core processing flow must not depend on UIA. Windows UIA is only an optional enhancement explicitly enabled by the user, not a default read channel and not a precondition for hook.

```text
所有平台
  默认：Hook 驱动的核心体验

Windows
  可选：用户启用 UIA → 使用 Windows UIA sidecar 增强读取能力

macOS / Linux
  只提供：Hook 驱动的核心体验
  不提供：UIA 开关、UIA sidecar、Windows UIA 调用路径
```

Therefore, a non-Windows build is not a "degraded version after the sidecar is missing": the UIA sidecar is not compiled or packaged, and Windows-specific implementations do not participate in compilation or linking. Windows targets always compile UIA-related code and ship the compiled UIA sidecar; "optional" only means the runtime does not start or use it by default, and that capability path is only taken after the user opts in.

Current isolation status:

- The Tauri shell's Windows-specific dependencies (`winvd` / `windows`) are confined to `[target.'cfg(windows)'.dependencies]`; the pin/fight-back in `window.rs` and `menu_window.rs`'s `SetForegroundWindow` are gated by `#[cfg(windows)]`, and non-Windows targets get a minimal substitute (tauri.conf.json `alwaysOnTop` + `set_focus`).
- core's UIA sidecar discovery (`paths::sidecar_exe`) is always `None` on non-Windows targets — not discovered, not started, not used; the sidecar client is pure std process-communication code with no call path on non-Windows targets (the Option chain degrades naturally, `sidecar_enabled=false`). The C# sidecar targets `net9.0-windows` and is published as self-contained win-x64, so it never enters non-Windows packaging (docs/sidecar.md §packaging).
- Residual verification boundary: `cargo check --target` for non-Windows targets needs a cross toolchain (`ring` pulls in a native C build via reqwest), which is not feasible on this machine; the `cfg(not(windows))` branches are minimal stubs whose correctness is guaranteed by review, with cross-compilation verification pending CI.

## Global wake hotkey

**Explicitly cut from 0.1.0** (docs/post-0.1.0.md): no global hotkey is implemented; the tray / gesture is currently the only wake path.

## Module split

`src-tauri/src/`:

| File | Responsibility |
|---|---|
| `main.rs` | thin assembly layer: three-window creation + pin, tray, core startup, IPC commands (including `ensure_card_window` / `close_card_window` window decisions, docs/case-runner.md §window decision hoisting) |
| `window.rs` | window pin (winvd) + z-order coordinator (bottom-to-top TOPMOST re-raise, docs/multi-window.md §Window Z-Order) — `cfg(windows)` gated |
| `tray.rs` | system tray (show/hide/exit) + CloseRequested hide-to-tray |
| `menu_window.rs` | settings panel popup / hide-on-blur; foreground focus on Windows via Win32 (`cfg(windows)` gated) |
| `tauri_runtime_actions.rs` | Rust shell-side runtime action layer (per-action effect recording for toggle_pet etc., docs/effect-reporting.md) |
