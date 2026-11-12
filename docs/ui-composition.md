# UI Composition

English | [中文](ui-composition.zh.md)

> What each Surface is belongs to concepts.md §Surface and [multi-window.md](multi-window.md); window creation, placement and coordinate units to [window-follow.md](window-follow.md); Card size derivation to [card-window-size.md](card-window-size.md); theme values to [theme.md](theme.md); module layering and data flow to [module-storage-flow.md](module-storage-flow.md).

## Principles

> **Scope of this document** — this document defines how UI code is layered and composed: the window component, the widget tiers, style composition, and the rules a component follows; it does not define what a Surface is, how windows are created or placed, how a Card's size is derived, or which theme values exist.

> **A component carries rendering only** — data logic stays out of it, and complex logic is extracted into a pure module or function that the component consumes. §Component principles states what this means for UI code.

## Layers

```
OS window          host        the shell creates, sizes, places, shows, hides and destroys it
Window component   component   one per window — its frame and mount convention; the entry of the component world
Surface content    component   the business component tree rendered inside it
widget tiers       component   atoms, composites and business components
DOM                rendering   what the user sees
```

- A window is a host object. A component never creates, sizes, moves or shows a window; that belongs to the window layer ([module-storage-flow.md](module-storage-flow.md)).
- The window component is the window's frame and mount convention: it renders the frame and exactly one Surface's content. The window's size is the host's decision ([pet-window-size.md](pet-window-size.md), [card-window-size.md](card-window-size.md), [multi-window.md](multi-window.md)); the component never asks for a size. A title bar is optional chrome, not a window property — a window that has one wraps its content in `Panel`; pet and the Cards Shelf have none, and a Card's title comes from its content.
- One Card component serves every container: the same Card renders inside its own window and inside a container surface.

## Window assembly

A window mounts exactly one component. Its services — bridge, store, theme, i18n, window adapter — are created by the entry, outside the component tree; no component creates one. A kind's host wiring — the adapter it may build on its own DOM, its Tauri listeners, its gesture actions — runs once the DOM exists and is started by the window component.

### Entry

```ts
// window entry — the only place that creates services
const shell = await createWindowShell("shelf"); // bridge, store, theme, i18n, adapter
mount(ShelfWindow, { target: document.getElementById("app")!, props: { shell } });
```

The entry mounts into the page root (`#app`), and the page root fills the window: the frame and the panels under it take their height from it. A root left at auto height collapses that chain — every `flex: 1` below it sizes to content, so a panel body neither scrolls nor clips.

### Window component

```svelte
<!-- ChatWindow.svelte — publishes the shell, renders, computes nothing -->
<script lang="ts">
  let { shell }: { shell: WindowShell } = $props();
  setContext(shellContext, shell);
</script>

<Window kind="chat" {shell}>
  <Panel title={t("chat.title")} onClose={() => void shell.adapter?.hide()}>
    <ChatPanel />
  </Panel>
</Window>
```

### Host

```rust
// host: the shell creates the window at the projected size; the page never resizes it
WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html#chat".into()))
    .inner_size(size.w, size.h)
    .build()?;
```

- `createWindowShell(kind)` is a module, not a component: it owns IPC, the store, theme and i18n application, and the window adapter. It is the only place a window's data logic lives.
- A kind's host wiring is a module in the shell layer (`shell/kinds/<kind>`): it registers the window's listeners, replaces the adapter when it needs its own DOM (the pet measures its rendered face), and exposes the gestures the window component forwards. The window component starts it after mount.
- The window component publishes the shell through Svelte context — its one act that is not rendering — renders the frame and the content, and computes nothing itself.
- A widget reads data from a prop or from the context and reports events through the callback it was given; it never creates a service.

## Widget tiers

| Tier | Owner | Examples | Rule |
|---|---|---|---|
| 0 behaviour primitive | bits-ui (dependency) | Select, Checkbox, Dialog, Tooltip | behaviour, accessibility, placement; no styling |
| 1 widget | this project | Button, Input, Panel, Select, Dialog, Tooltip | variants are defined here; widgets may compose each other |
| 2 business component | this project | ChatPanel, MenuPanel, ConfigRow | composition and business semantics only |

- A tier-1 widget either wraps a tier-0 primitive with this project's variants, or is written here when bits-ui has no counterpart.
- Dependency is one-way: 2 → 1 → 0; a tier-2 component does not use a tier-0 primitive directly.
- Variants belong to tier 1. A tier-2 component composes and places; it defines no appearance rule of its own.
- A widget's height is a declared layout constant, never content-driven ([card-window-size.md](card-window-size.md) §Content blocks).
- Something becomes a widget when it appears in more than one place or carries a behaviour or accessibility contract; a one-off layout stays inline.
- `Panel` renders a title bar and a close button; what closing means is the caller's callback — hiding a window, dismissing a Card — not the widget's decision.

## Style composition

| Layer | Source | Produces |
|---|---|---|
| token | the `--ov-*` table (the theme's landing point) | CSS custom properties |
| semantic utility | the Tailwind theme mapped from the tokens | utility classes |
| primitive state | bits-ui data attributes | state selectors |
| variant | tailwind-variants | class functions and slots |
| motion | Svelte transitions | `transition:`, `animate:flip` |
| instance override | the caller's `class` | a local adjustment |

Discipline: a utility reads tokens and never a literal value; a variant uses utilities and primitive state; an instance override adjusts and never redefines; the theme layer carries no size ([card-window-size.md](card-window-size.md) §Recompute triggers). A scroll container wears the theme's scrollbar — a thin bar with the theme's thumb colour (`--ov-scrollbar-thumb`) — because the platform scrollbar does not follow the theme and paints a light track across the dark panels. In a transparent window the panel silhouette is the visible boundary: a layer that fills the window — the modal dim — carries the panel radius (`--ov-panel-radius`), or it paints square corners over the desktop. Text selection follows the chrome/content split: chrome — titles, buttons, labels — is not selectable; content — message bodies, Card text, code — is.

## Component principles

A component carries rendering only; the rules below are that principle applied to UI code:

- A component does not call IPC, the positioning engine, storage or the Harness; data arrives through props and leaves through callbacks.
- Inputs are props (data) and snippets (structure); outputs are DOM and events.
- A component never derives size; size comes from the size model ([card-window-size.md](card-window-size.md)).

## Naming and files

- File names are semantic and multi-word; one widget per file, with its variants beside it.
- A component file carries the component; shared pure logic lives in a `.ts` module.
- A widget's public surface is its props and snippets; callers do not address its internal elements by class name.
