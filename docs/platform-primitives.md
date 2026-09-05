# Platform Primitives Design

English | [中文](platform-primitives.zh.md)

An abstraction group for platform-specific capabilities (not called adapter): OS-level capabilities such as virtual desktop switching that are reused across terminals and Components.

> See `concepts.md` §15 (Platform Primitives) for the conceptual positioning. This document defines the interface capabilities and the per-platform implementations.

## Capability Interface

platform-primitives is an instantiable kind of thing — expressed as a Rust trait, with one implementation per platform:

```rust
pub trait PlatformPrimitives: Send + Sync {
    /// Switch to the virtual desktop of the target window (shared by read precondition / jump)
    fn switch_vd(&self, hwnd: i64) -> bool;
}
```

## Consumers

- **terminal-adapter** (docs/terminal/terminal-adapter.md): when the target window is invisible (cloaked) during a read → switch desktop and then read. Switching is an interruptive action and must be gated by explicit consent through fetch_terminal's `vd_switch` (docs/agents/claude/hook.md §VD switching capability); the adapter never switches automatically

## Implementation

| Platform | Implementation | Capability |
|---|---|---|
| Windows | COM (`IVirtualDesktopManager`) | Virtual desktop switching |
| (Other platforms as needed) | — | — |
