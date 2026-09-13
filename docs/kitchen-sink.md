# Kitchen Sink

English | [中文](kitchen-sink.zh.md)

> The component contract it renders belongs to `docs/ui-composition.md`; theme values to `docs/theme.md`; the Component protocol to `docs/components.md`; behaviour verification to `docs/case-runner.md`.

## Principles

> **Scope of this document** — this document defines the rendering-layer verification surface: the page, what it renders, its controls, and the guarantees that keep it in sync with the product. It does not define the component contract, the theme value fields, or the Component protocol.

> **Observation does not act** — the surface wires no application action: no IPC, no window management, no storage write, no drag. A page that can change state is a second entry point, not a verification surface.

> **No colour of its own** — the page introduces no colour: every surface it paints comes from the theme in use, and the theme selection is the only colour input. A value that is not a theme token is a defect, not a detail.

> **Everything is laid out flat** — no option may hide behind a control: no dropdown, no menu, no state revealed only on hover. Every choice shows all of its alternatives at once, and every operation takes effect in one click. Nothing floats either: nothing hovers above the content — no pinned bar, no floating layer, no overlay; everything sits in the flow.

> **The surface holds no copy** — everything it shows is production code and live data: the components are the product's own components, Cards render through the Component renderer, token values are read from the document root, and the lists it enumerates come from the product's own registries.

## Coverage: the rendering layer the case runner leaves open

`docs/case-runner.md` §Observation boundary excludes the rendering layer — DOM construction, CSS and the size computation stay outside its observation scope, and a projected Card size enters a case as data only. The kitchen sink is the surface for exactly what stays out: the components as they paint, the token table as it resolves, and the states a surface can take.

The two surfaces are complementary rather than overlapping. Behaviour — replay, window decisions, lifecycle, the belief state — is observed headlessly by the case runner; appearance — DOM, CSS, tokens, states — is observed here, and in the form that ships.

## Layout

The page is one reading in two tiers: the surface tier first (the surfaces are what the product is judged by, and the spec tier is the material behind them), each filling one viewport. Whatever exceeds that viewport **scrolls inside its own tier**, so the tier boundary is only ever pushed up during a transition — **at rest the screen holds exactly one tier**. The right edge carries one dot per tier: the current position, and the way to jump.

```text
┌ tier 1 · surfaces ─────────────────────────────────────── exactly one screen ┐
│ theme ▪▪▪▪▪▪▪▪▪        language [中][EN]                                     │
│  scrolls in place: the boundary stays above the viewport                     │
│  panels at their real sizes + the Card types                                 │
│                                                          tier ●○             │
│        ↓ reach this tier's bottom, then one notch more: the whole tier       │
│          slides up and the next one arrives from below                       │
├ the boundary only moves during a transition ─────────────────────────────────┤
│ theme ▪▪▪▪▪▪▪▪▪        language [中][EN]                                     │
│  spec content (keeps scrolling inside this tier while it exceeds the screen) │
│                                                          tier ○●             │
└──────────────────────────────────────────────────────────────────────────────┘
```

The two choices that apply to the whole page — the theme and the interface language — are one header rendered at the head of each tier. It is not pinned: it scrolls away with that tier's content, and the next tier opens with the same header again, so the controls sit where a tier begins rather than following the reader, covering content, or leaving the flow. Both instances read one model, so switching in either one switches the page.

The pager drives the same tier transition, and the page pulls in no scrolling library.

## Scrolling

The page is a document: its height is its content, the scrollbar is the product's own and sits at the window edge, and each tier is at least one screen tall (`min-height`, so a longer tier grows). Displacement belongs to one module, `reading-motion`, and it runs two curves, each with one job.

**The wheel tail is exponential.** The wheel pushes a target position; the screen follows it every frame with `k = 1 - 0.001^(dt / TAU)` and `TAU = 0.7s`. The coefficient is recomputed from the real frame delta, so the tail takes the same time on any refresh rate. Touch and the keyboard emit no wheel events, so their displacement stays native scrolling, and the engine takes the target back from the position it finds.

**A tier change is a timed glide.** When one wheel notch would push the target across a tier boundary, that notch stops being an ordinary step: the position is carried to the boundary over `RAMP_SECONDS = 0.5s` of cubic ease-out — down puts the next tier's start at the top edge, up puts this tier's end at the bottom edge. A fixed duration over a computed landing means it stops exactly on the boundary, and a reader who reverses during the glide cancels it and gets the position back. While the glide runs it takes no input, so the rest of a fast gesture cannot push the position off the boundary again.

**Where the crossing is decided.** The check runs in the wheel handler, once per notch, and compares the tier index of the target before and after the step — not the position and not the landing distance. Deciding it every frame instead would snap as soon as the viewport touches a boundary, dragging the reader off the end of a tier they were still reading. A reader who reaches the boundary zone without the wheel (touch, keyboard) is corrected by the same per-frame reset, which fires on the first frame the viewport straddles a boundary and no glide is running.

```ts
// reading-motion — the crossing test: once per notch, comparing the tier of the target before and after
const push = (delta: number): boolean => {
  const sections = tiers();
  if (!sections.length) return false;
  const up = delta < 0;
  const from = motor.target;
  const to = Math.max(0, Math.min(maxDoc(), from + delta));
  const fromSeg = sectionIndexOf(from, sections);
  const toSeg = sectionIndexOf(to, sections);
  if (fromSeg === toSeg) {                            // same tier: the wheel tail advances it
    motor.target = to;
    return false;
  }
  const boundary = up ? sections[fromSeg].offsetTop : sections[toSeg].offsetTop;
  const landing = Math.max(0, Math.min(maxDoc(), up ? boundary - viewH() : boundary));
  startGlide(landing, clock(), "越过第 " + (up ? fromSeg + 1 : toSeg) + " 段边线");
  return true;                                      // tier change: this notch becomes a glide
};
```

`reading-motion` holds the whole mechanism; the page only mounts it. The engine asks the page for the tier elements on every frame rather than capturing them once, because the page re-mounts its content on a language switch and a captured node would go stale.

Scrollbar appearance is not this page's business: it inherits the application's global rule (a thin bar with the theme's thumb colour), so a scrollbar is judged here as the product draws it.

## Structure

```text
kitchen-sink.html ──► Kitchen Sink entry ──mount──► KitchenSink.svelte
                                                       │
                                    ┌──────────────────┴───────────────────┐
                                    │                                      │
                              header                                 pager
                              one model, once per tier       two tiers + dots
                                    │                                      │
                                    └────── both only compose blocks ──────┘
                                                       │
      ┌─────────────┬──────────────┬────────────┬──────────────┬─────────────┐
  TokenBoard   WidgetBoard     RowBoard    PanelBoard     CardWall
      │             │               │           │             │
  source of     hand list       hand samples  state stubs  source of
  truth: the    (tier-1         (one config   (chat / menu truth: the
  theme's       widgets and     row per       / shelf as   Component
  known-token   their states)   field kind)   the panels   type union
  table                                       receive)
```

A block carries rendering and nothing else; each block names its own source of truth in the diagram, which is what makes the page unable to drift.

## Extension

| Grows by | It appears in the page | What enforces it |
|---|---|---|
| a token | automatically | the token board walks the theme's known-token table, and the token guard keeps that table and the `:root` table in step |
| a Component type | automatically, and the build fails first | the sample table is keyed by the Component type union, so an unrepresented type is a compile error; the wall walks the sample table |
| a field or a composition form inside a Component | nothing to do | a sample is one instance per type, not a field catalogue; a composition form is one more sample |
| a window panel | by hand — one row in the panel block plus the state stub it needs | the coverage line names the panels that are rendered |
| a tier-1 widget | by hand — one section in the widget block | the coverage line compares the widget files against the list the page claims |
| a reading (layout) | by hand — one file plus one manifest row | — |
| a theme | automatically | both page headers enumerate the theme list |

How a new Component type propagates: the protocol gains a type, the frontend's type union gains it, and the type check then refuses to pass until the sample table carries an instance of it — at which point the wall shows it without being edited. The wall is not a hand-written list of cards; it is a walk over that table, so it can never omit a type that the product can render.

## Route

The Kitchen Sink page is an HTML entry of the frontend package, reached in both forms by that entry's path: on the dev server while iterating, and as a built page in a browser when a style conclusion has to hold in the CSS pipeline of the packaged build. The application entry does not know this page — its routing never dispatches to the page, and the Tauri form declares no window for it.

## What it renders

- Every token of the theme's token table, with the value it resolves to under the current theme.
- The tier-1 widgets with their variants and their states — disabled, error, read-only.
- The tier-2 components with representative data: configuration rows of every field kind, provider key rows in all three states, and the window panels at their real sizes.
- Every Component type, rendered through the Component renderer.

The modal surfaces are **not** shown here: a dialog, a popup and a tooltip are floating layers by construction, and this page floats nothing. What a modal contains is judged from the components it is built on, and the shells that host it carry its behaviour.

The states are as much part of the content as the components are: a surface that only shows the happy path does not show the surface. Empty states, the offline and read-only degradations, a restart banner, an error row, an over-long title, a Card at its height cap — each is a rendering outcome the product has, and each is judged here.

## Controls

- **Theme and language** — one model, rendered at the head of each tier, so both tiers carry the same controls and switching in either one switches the page. Each is **a row of small boxes rather than a text dropdown**: the theme row draws the palette of the theme in use — ground, panel, text, accent, state colours, bubble, chart — so what is shown is the colour scheme itself and not its name, and the language row shows the languages themselves. Only the built-in theme exists today; a second one is theme work, and this page presets no palette of its own. Every string in the page is the product's own `t()` output, so switching the language re-words the specimens rather than translating a copy.

## Isolation

The Kitchen Sink page must not reach the application bundle: the Tauri shell embeds that bundle, and a verification surface is not a product surface — its demo theme tables and its own stylesheet would otherwise ship as part of the app.

Isolation is by **entry**, not by a dev flag. The application entry and the Kitchen Sink entry are two separate HTML entries of the same frontend package, and the page build writes to its own output directory; neither entry references the other, so nothing in the page can be pulled into the application bundle, and nothing of the application's window world is needed by the page.

The mechanism: Vite's `kitchen-sink` mode points the build at the page entry, its own output directory and a relative base, and the preview serves that artifact by reading the same mode — the entry self-starts, so no application route knows this page.

```text
npm run kitchen-sink:dev      → dev server; both entries are live, the page at /kitchen-sink.html
npm run kitchen-sink:build    → kitchen-sink.html + its assets, into dist-kitchen-sink/
npm run kitchen-sink:preview  → serves that artifact (the same mode, not an extra flag)
```

A dev flag isolates nothing: it relies on dead-code elimination along one shared entry, and the moment the flag is opened — which is exactly what reading the page under the packaged build requires — the page's chunk and its demo theme tables land in the application bundle.

The Kitchen Sink page still answers that question, because its own build runs the same CSS pipeline: opening the built page in a browser confirms a style conclusion in the form that ships, without putting the page inside the installer.

Two assertions keep the isolation honest: the application output carries no Kitchen Sink string or chunk, and the page output carries no window entry of the application.

## Sync guarantees

- A new token shows up in the token list, because that list is the theme's known-token table.
- A new Component type fails the type check until the page represents it, because the sample table is keyed by the Component type union — completeness is enforced at compile time, not by review.
- A component change shows up without the page changing, because the page composes the component instead of restating it.

## What stays outside

Real desktop behaviour: multi-monitor and mixed-DPI placement, window creation by the shell, tray and OS-level interaction, and cost under load. Those are read on real hardware in the packaged form, not here.
