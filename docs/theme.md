# Theme Design

English | [中文](theme.zh.md)

> - **Scope of this document** — this document defines what a theme is, its application scope, the export and sharing format, and Config access; the theme's field list and naming are converged from the actual UI needs and are not prescribed here, and the token table they land on lives in `packages/apps/src/styles/tokens.css`.

## Definition

A theme is a complete visual scheme: a color table as its main body, plus common style modifications that change together with the visual. A theme is not a single color switch, not scattered CSS overrides, and not a partial skin of some Component.

Config provides formal facilities for themes:

```text
theme   the name of the theme in use
themes  Map of theme name → theme value
```

Each theme value expresses the theme's color table and common style modifications. The concrete field list, naming, and validation rules should be converged from the actual visual needs of the current UI; this document does not prescribe them. The converged physical landing point is the `packages/apps/src/styles/tokens.css` `:root` `--ov-*` token table — theme value fields correspond one-to-one with that table, and applying a theme overwrites that table.

## Application scope

A theme is the single visual choice for the entire app. The user's intent when switching `theme` is for the whole app to use this visual scheme immediately:

- All currently visible Surfaces and transient popovers adopt the new theme immediately; windows opened later also adopt it.
- There is no state where Chat, Card, Menu, and pet each choose their own theme independently.
- Theme switching is a purely visual change: it must not alter window open/close state, position, size, layout memory, Chat reading position or input content, Card content and visibility, pet name, expression, or Harness behavior.

## Export, sharing, and compatibility

Themes can be exported as shareable standalone files. The export payload must carry the **Config version** at generation time, so the importer can interpret the theme according to the declared config generation instead of guessing field meanings.

```text
exported theme file (config_root/themes/<name>.theme.json)
├─ config_version
├─ name
└─ value (one complete theme value: the token override table)
```

Import must go through a compatibility layer: first transform the theme into a form the current version understands according to the Config version declared by the exported file, then validate and write it into the local `themes` Map. Normal local updates still keep atomic rejection; a failed import does not change the currently active theme or the existing theme table, and gives the user a clear reason.

Compatibility only promises evolution from known old versions to the current version. When an old app encounters a future Config version it does not recognize, it must explicitly reject the import and prompt the user to update the app; it must not guess, truncate, or silently apply a future theme.

Theme export is self-contained: it may only depend on its own complete value, and must not reference, inherit from, override, or require the importer's other themes, the current theme, any external Config fields, or the machine environment. The import result in any compatible app is determined only by the export file itself and its declared version.

Physical entry: the theme section at the bottom of the settings panel (menu) provides "export current theme / import by filename"; import goes through the `import_theme` command → version check → compatibility transform → validation → unified modification pipeline, writing `themes.<name>` (atomic rejection + config_changed broadcast, all windows switch immediately). For token names and validation rules see `core/src/config.rs` `validate_theme_table`; for the frontend application side see `packages/apps/src/theme.ts`.

## Config access

Themes use Config's default access rights: both the local user and the LLM can read the current `theme` and `themes` Map, and both can modify them through their existing Config entries. LLM reads and writes of themes continue to be constrained by the existing query → update, validation, persistence, and audit pipeline; no separate permission model is created for themes.
