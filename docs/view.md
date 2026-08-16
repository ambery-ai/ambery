# View Design

English | [中文](view.zh.md)

> See concepts.md §3 for the concept definition. This document defines the physical implementation and interaction details; tradeoffs not specified in concepts are recorded here.

## Config fields

| Field | Takes effect | Agent access | Behavior |
|---|---|---|---|
| `view_scale` (legal range [0.2, 4.0]) | hot | visible, modifiable | immediately recomputes pet size, center anchoring, and obstacle area |
| `badge_style` | hot | visible, modifiable | immediately updates the unread badge style |
| `badge_side` | hot | visible, modifiable | immediately updates the unread badge side |

## Name

The pet name is a stable identity value in formal Config (field `name`). All UI that needs to address pet and subsequent Harness identity copy read the current name; existing Chat history and already-generated Cards are not rewritten.

- On first Config initialization the formal default name **`Ambery`** is written (the default name is not language-specific, and the name itself is not translated).
- After initialization, the name is independent of UI / Harness language: switching any language later never auto-renames.
- The name is not marked `no_llm_visible`. Both the local user and the LLM can explicitly read and modify it through their existing Config entries; LLM modification remains constrained by the existing query → update, validation, persistence, and audit pipeline. Validation: non-empty, ≤ 64 characters.
- There is no "reset the default name according to the current Harness language" operation; language switching is not a rename operation.
- Harness identity copy reads the current name via the `{name}` placeholder: the built-in default in `base_prompt` and the identity line of the default AGENTS.md carry the `{name}` placeholder, which is replaced with the current `name` when assembling the request header (the built-in base_prompt text is upgraded to the placeholder version at load time; user-modified text is kept as-is). On the UI side the chat title and placeholder read the current `name`.

## Form

- **Tauri mode**: borderless, transparent-background, always-on-top horizontal oval window; the window contains only the kaomoji and no other UI elements.
- **Browser test mode**: a `position: fixed` DOM element simulates the same window and keeps behavior consistent with Tauri mode, for Chrome DevTools testing of display logic.

The two modes share the same gesture and event set; the difference is only in the drag/coordinate driver layer.

## Gestures and Chat summoning

pet has **no snap state** (edge snapping is OS-style raw docking and conflicts with this app's own window-placement engine):

- **Right-click** = summon/close Chat (dispatches `chat:toggle`; for the Chat Panel see docs/chat-panel.md). pet stays in place — no teleport, and the floating animation is unaffected.
- **Left-click drag** is always available (no lock state).

## Dragging

- Browser mode: pointer events (pointerdown/move/up) update left/top.
- Tauri mode: calls the window drag API (`startDragging`).

## Component anchoring

Components are anchored to the View center and pop up offset in the specified direction (the direction is specified by pet via `call_component`). When the View moves, already-popped Components follow at the relative pet offset (docs/window-follow.md: engine occupied area + layout memory; the `auto` direction is computed on the spot by the engine from the remaining screen space). For direction geometry details see docs/components.md.

## Surface entry (pet gestures)

| Gesture | Destination |
|---|---|
| Left-click drag | move pet (spatial position expression) |
| Right-click | summon/close Chat (`chat:toggle`) |
| Middle-click | enter Cards Shelf (`shelf:toggle`) |

Cards Shelf is the transient management popover for the Card collection (`shelf` static window, no title bar — a context-menu-style list; **not a Surface**, see §consistency analysis): each living Card is one row (type icon + title + show/hide / dismiss icon buttons), actions = visibility toggle and dismiss. Visibility toggle writes `_meta.user_closed` (`set_card_user_closed` IPC) and lets pet open/hide the corresponding Card window via `shelf:visibility`; dismiss goes through the closed_by_user two-line event + deletes `.card.json` + pet destroys the window.

Shelf does not participate in Card layout: it does not enter the engine occupied area, does not follow pet, is not draggable, and has no layout memory. It is a transient management panel anchored to pet.

- **Size** = pet's current physical size ×3 (3:1 ratio, computed on open; clamped to 180–480 × 120–240 to prevent distortion under extreme scale)
- **Position** = the lower-left corner falls on pet's center and extends to the upper right (occludes pet, clamped by screen bounds)
- **Toggle**: middle-click toggles — opens when closed; when open, middle-clicking pet or anywhere on the shelf closes it directly
- **Close**: middle-click / close on blur (600ms arming delay) / pet drag or tray closes it as collateral; no title bar and no ×
- Shelf itself has no userClosed state — under transient semantics, closing leaves no visibility or layout trace

#### ⟡ Consistency analysis

pet is the anchor through which users enter the Surface world, not a member of it: left-click drag expresses spatial position, right-click enters Chat, middle-click enters Cards Shelf. Chat is the Surface for conversation content, Card is the Surface for persistent work artifacts; both share display, hide, and restore semantics (engine occupied area, persistent spatial layout, and following). Neither Cards Shelf nor Menu is a Surface — they are transient popovers anchored to pet / shell (close on blur, do not enter the engine occupied area, no layout memory or persistent visibility): Menu is the settings entry, Cards Shelf is the Card management entry, and their truths live in Config and `.card.json` respectively, not in the popover itself.

## Events

| Event | Payload | Description |
|---|---|---|
| `chat:toggle` | `{}` | right-click summons/closes Chat |
| `view:moved` | `{ x, y }` | center coordinates after dragging ends (used for Component anchor calculation) |
