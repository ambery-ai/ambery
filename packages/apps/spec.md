# Spec — packages/apps

English | [中文](spec.zh.md)

## Technology choices

| Form | Technology |
|---|---|
| tauri | Tauri 2 shell + vanilla TypeScript frontend |
| webui | pure-web form, same frontend code's second host |

Tradeoffs:

- **One frontend, two hosts** — window management and IPC differ per form; the frontend code does not. The bridge layer (bridge.ts / effects) is the seam; hosts implement it, the UI stays host-agnostic.
- **Vanilla TS, no framework** — display logic is simple; a framework's abstraction cost outweighs its benefit. Browser debug mode runs vite directly; UI is testable with Chrome DevTools.

## Architecture decisions

1. **Each form is one package over the same frontend core**: tauri and webui both embed/serve the same src; they differ in host layer (window management, IPC transport, packaging).
2. **Forms talk to core only through the established channel** — native IPC in the packaged tauri form; thin HTTP+WS loopback (127.0.0.1) in browser/webui mode. Same frontend code in both modes.

## Fixed constraints

- No UI framework; vanilla TS.
- UI interaction must not use browser-native popups (alert / prompt / confirm): errors and input are expressed with in-app UI elements.
