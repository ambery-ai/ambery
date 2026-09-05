# Config

English | [中文](config.zh.md)

The Config domain (concepts §7, alongside the Storage domain) consists of `config.json` and `AGENTS.md`; the on-disk directory is determined by `config_root()` in `core/src/paths.rs`.

## Core Model

```text
config.json (historical payload)
  → version dispatch / migration
  → null normalization + recursive reconcile
  → Config (runtime truth)
  ├─ local schema projection → CLI / settings panel
  └─ LLM restricted projection → edit_config
```

**The `Config` type is the single source of truth for the current configuration structure.** Field types, descriptions, defaults, migration rules, and consumer-facing access metadata should all be colocated with the field definitions. serde is responsible for persistence, schemars for type and UI schema, and the project `config` metadata for version evolution and consumer access scope.

The file-level `version` does not enter `Config`: the load pipeline reads it, and save injects it. It is a monotonically increasing integer; historical files without the field are v0.

### Config path grammar

All addressable Config path segments uniformly use the lowercase ASCII grammar: `^[a-z][a-z0-9_-]*$`. It guarantees that dotted paths are unambiguous and stable across entry points; uppercase variants, Unicode confusable characters, `.`, whitespace, and emoji are not allowed. Values such as `face` may keep arbitrary Unicode; this rule only constrains path keys.

- Static `Config` / object field names (including serialized names after serde rename) are checked by the `ConfigMeta` derive at **compile time**;
- dynamic keys of maps are checked by unified validation at **runtime** at load, after migration/reconcile, and on every update;
- keys produced by map defaults must also pass the runtime check;
- dynamic key validation follows the same failure semantics as other validation: updates aggregate errors and reject atomically; loading aggregates errors, writes a load report, and does not block startup. When a dynamic entry's key cannot be defaulted, the repaired result is that the entry does not exist; the remaining map entries are preserved.
- `version` is a file-level control field, not part of the Config descriptor tree, and this grammar does not apply to it.

```text
config_root/                   # Windows: %USERPROFILE%\.config\ambery\
  config.json                  # Config persistence (Config::save, pretty JSON; save injects version)
  config.bak/config-v0NN.json  # symmetric backup before a version replacement
  AGENTS.md                    # identity prompt; Harness::load bootstraps a default when missing
  storage/                     # Storage domain, see docs/storage.md
```

The API key itself exists only in the environment (the app-level env file or process environment — see docs/llm-setup.md §Key storage model); config only stores the variable name; keys are never written to config.json.

### LLM Group

The provider profile fields `base_url`, `model`, `api_key_env`, `temperature`, `context_window`, `compression_reserve` are all profile-level fields. `context_window` is a fact about the model window, not a global policy; the Compression trigger point is `context_window − reserve`, where the global `compression_reserve_default` is used when a provider does not set `compression_reserve`. Without `context_window`, no compression occurs; the only effective entry point is `effective_compression_limit()`, and the measurement uses the true usage token count rather than a chars/4 conversion.

**Default preset `api_key_env` values follow the `AMBERY_<NAME>_API_KEY` convention** (`AMBERY_DEEPSEEK_API_KEY` / `AMBERY_MOONSHOT_API_KEY` / `AMBERY_ZHIPU_API_KEY` / `AMBERY_OPENAI_API_KEY`; ollama has none — local endpoint, no key). `api_key_env` is editable per provider (a private provider may point at its own variable).

**`llm.active` has an explicit unconfigured value** as a legal option alongside `debug` and the provider keys (dynamic enum: `["unconfigured", "debug", ...provider keys]`). It is the default of a fresh install: the runtime treats it as "no LLM" (docs/llm-setup.md §Backend changes).

The LLM tool cannot access the `llm` subtree (see [Reflection and consumer projections](#reflection-and-consumer-projections)); this does not change the local runtime semantics above.

### Kaomoji Pool

The kaomoji domain is a fixed object; the two pools are its fixed fields; only kaomoji names inside a pool are dynamic map keys:

```rust
struct Config {
    #[config(validate = [Func(validate_kaomoji_pools)])]
    pub kaomoji: KaomojiConfig,
}

struct KaomojiConfig {
    pub system: HashMap<String, KaomojiEntry>,
    pub user: HashMap<String, KaomojiEntry>,
}
```

| Path | Ownership and permissions | Purpose |
|---|---|---|
| `kaomoji.system` | System pool; visible to the agent and updatable as a whole | System-state kaomoji; the window-size scan only scans this pool; do not modify by default |
| `kaomoji.user` | User pool; visible to the agent and updatable as a whole | User-defined kaomoji; may initially be empty |

When the agent creates, modifies, or removes kaomoji in either pool, it does not use the `add` / `delete` actions; instead it first reads the complete current map of the target pool via `query(path="…", view="object")` and then writes back the complete new map via `update`. The `desc` of the `kaomoji.system` node must clearly state "do not modify by default"; the only difference between the two pools is kaomoji ownership and the scan source for window-size scans, not LLM access permission.

`validate_kaomoji_pools` validates two invariants of the final kaomoji domain:

```text
keys(system) ∩ keys(user) = ∅
{ idle, processing, notify } ⊆ keys(system) ∪ keys(user)
```

Keys are globally unique across the two pools, so there is no implicit priority of user overriding system. The user can atomically move kaomoji between the two pools via the local settings panel; the complete Config after the move must pass unified validation. Base states can be moved between the two pools and still participate in default states and per-key resolution in `set_autonomy(key)`; the system pool's additional responsibility is only being the scan source for window-size scans.

## Field Metadata

The target syntax uses the same project attribute to carry field semantics; the concrete derive implementation is responsible for making `#[config(...)]` a legal attribute and for generating the descriptor tree shared by loader / reflect / tool.

```rust
#[derive(Serialize, Deserialize, JsonSchema, ConfigMeta)]
#[config(migrate = [
    3..=3 => Func(migrate_config_v3),
])]
pub struct Config {
    #[serde(default = "default_timer_batch")]
    #[config(migrate = [
        0..=2 => Default,
    ])]
    pub timer_batch: usize,

    #[config(no_llm_visible)]
    pub llm: LlmConfig,
}
```

The semantics of `no_llm_visible`: the node and its entire subtree do not enter the LLM's Config projection. Therefore `edit_config` cannot grep, query, or update it; the local CLI, settings panel, persistence, and LLM backend selection at startup are unaffected. The two kaomoji pools are not marked `no_llm_visible`; both are visible to the agent and updatable as a whole.

It is not `serde(skip)`: the latter would make the field no longer part of the persisted Config and would also make it disappear from all schema consumers.

### Validation enum

Validation validates **whether the final current Config is allowed to take effect**, which is different from migration's "how a historical value becomes the current value". All validators run after null normalization, reconcile, and serde structural validation.

```rust
enum Validation {
    Range { min: ..., max: ... }, // fixed numeric boundaries
    OneOf([...]),                  // fixed candidate set
    Func(ValidationFn),            // dynamic candidates or cross-field invariants
}

type ValidationFn = fn(&Value) -> Vec<String>;
```

`Range` and `OneOf` only express static rules; dynamic legal values must use `Func`, for example whether `llm.active` belongs to the current provider keys. `Range` may only be attached to statically known numeric nodes; the `ConfigMeta` derive must reject at compile time attaching it to string, bool, object, map, or other non-numeric nodes. `Range` uses closed intervals; `min` / `max` are both optional: `Range { min: Some(0), max: Some(1) }` means `0 ≤ value ≤ 1`, only `min` means a lower bound, only `max` means an upper bound. `OneOf` compares by strict JSON value equality; no case folding, trimming, numeric-string conversion, or other normalization. `Func` receives the final value of the node it is attached to, returns only messages, and does not carry a path; the framework fills in the complete path of the mounted node. It may return multiple messages.

Parent and child validators can run in combination: a child node validator validates itself or its own subtree; a parent node validator validates the final subtree. Each update only runs the validators of the target node's subtree and its ancestor validators, in subtree→ancestor order; loading has no single target, so all validators run. Errors are aggregated stably by the complete path of the validator's mounted node in lexicographic order, and by message lexicographic order within the same path.

`Range` and `OneOf` are part of the descriptor / reflect output, allowing the CLI and settings panel to mechanically choose controls; they are only a read-only projection of the same validation truth, and the backend remains the sole gatekeeper. `Func` outputs neither functions nor pseudo-static constraints; after submission, unified validation returns its messages.

Any validator failure in an update rejects the entire update; neither the in-memory nor the on-disk Config is changed. Validator failure at load does not block startup: all messages for the same node are written to the load report at once, that node is defaulted only once, and processing then continues with the remaining nodes.

## Version and Migration

Load flow:

```text
Read raw JSON payload + version
  ├─ version == current: null normalization → recursive reconcile
  ├─ version <  current: migration → null normalization → recursive reconcile → backup → write back current
  └─ version >  current: back up the new-version site → find an available old backup for read-only load; refuse startup if no backup exists
```

migration is a **pure, deterministic, replayable mapping from historical JSON to current JSON**; it must not read the environment, network, files, or other runtime state. Functions do not receive `version`: the applicable source version is already explicitly expressed by the range to which it belongs.

### Migration enum

A node may declare a table containing multiple, non-overlapping ranges:

```rust
enum Migration {
    Default,
    Rename { from: Path },
    Func(MigrationFn),
    RenameWithFunc { from: Path, func: MigrationFn },
    // missing an explicit range = implicit Current
}

type MigrationFn = fn(Value) -> Result<Value, ConfigMigrationError>;
```

| Rule | Input source | Result |
|---|---|---|
| `Default` | none | the current node's default |
| `Rename` | the complete old dotted path specified by `from` | becomes the current node value verbatim |
| `Func` | the subtree at the current node's same path in the old JSON | the result of `func(old)` |
| `RenameWithFunc` | the complete old dotted path specified by `from` | the result of `func(old)` |
| implicit `Current` | the subtree at the current node's same path in the old JSON | kept verbatim, then reconciled |

`Rename`'s `from` is always a **complete old dotted path**, so renames and cross-object moves have no implicit "relative to which parent" convention.

```rust
#[config(migrate = [
    0..=2 => Default,
    3..=4 => Rename { from: "legacy.timeout_ms" },
    5..=5 => RenameWithFunc {
        from: "legacy.timeout_seconds",
        func: seconds_to_ms,
    },
    6..=7 => Func(normalize_timeout_v6),
    // v8 onward missing: implicit Current
])]
pub timeout_ms: u64;
```

### Historical Coverage and Parent-Child Relationships

Missing a rule does not mean "guessing": it has a unique, explicit `Current` semantics. But for historical versions where the current path did not yet exist, it cannot fall into implicit `Current`; that segment of history must be explicitly handled by the node itself or by an ancestor's `Default`, `Rename`, `Func`, or `RenameWithFunc`.

Parent and child nodes may both declare migration metadata; the key constraint is that **explicit ranges must not intersect**. The check covers all enum variants, not only functions:

1. Ranges within a single node's table must not overlap;
2. Explicit ranges of any ancestor and descendant node must not intersect;
3. The `Config` root follows the same rule;
4. "When the current path began to exist" is a historical fact that the compiler cannot infer from the current type; every released version must be verified by migration fixtures.

Classic example (currently v11):

```rust
#[config(migrate = [
    3..=3 => Func(migrate_config_v3),
])]
struct Config {
    root: Root,
}

struct Root {
    #[config(migrate = [
        4..=7 => Default,
        // v8..=10 missing: implicit Current
    ])]
    leaf: Type,
}
```

| Source version | Effective rule | Meaning |
|---|---|---|
| v3 | root `Func(migrate_config_v3)` | the root function owns the complete Config migration for v3 |
| v4–v7 | `Default` of `root.leaf` | `leaf` had no preservable semantics in these versions |
| v8–v10 | `leaf`'s implicit `Current` | read the historical value at the same path |

This example shows: parent-child metadata can coexist; only explicit rules for the same source version conflict, so there is no need to specify an execution order for parent and child functions.

Complex transformations are handled by attaching `Func` to a sufficiently high node: a single field attaches to the leaf; multiple sibling fields attach to their nearest common parent object; cross-top-level fields attach to the `Config` root. The root function receives the complete Config payload (excluding the file-level `version` control field).

### Failure and Deletion

**Any migration failure uniformly defaults that node, reports it item by item, and continues loading.** This includes `Rename` not finding its input, `Rename` input having an invalid type, `Func` / `RenameWithFunc` returning `Err`, etc. The report contains at least the target path, source version, and reason.

`Delete` is not part of the `Migration` enum: a deleted old field is no longer in the current `Config`, so there is no place to attach metadata. After all mappings complete, recursive reconcile removes old paths that do not exist in the current schema and reports them item by item; old paths read by `Rename` / `RenameWithFunc` are also cleared in this phase.

## Default, null, and Recursive Reconcile

### Default Sources

default is configuration semantics, not a technical zero value casually guessed from the Rust type.

| Node type | default rule |
|---|---|
| Leaf | **must** declare its own semantic default |
| Static object | may omit; when missing or invalid, recursively construct the defaults of its static children |
| map field | **must** declare its own semantic default |

Therefore a static object does not need to repeatedly maintain an object-level default; its default shape is assembled from its static children. A leaf cannot continue constructing downward, so it must have a default. A map's keys are dynamic, and when the whole map is missing there are no known child keys to construct downward, so the map itself must define its default.

A map default may be an empty map or a system preset; it is decided by the field's product semantics and cannot be automatically replaced by `HashMap::default()`. For example, when `kaomoji.system` is entirely missing it can recover the system kaomoji mapping, and when `providers` is entirely missing it can recover the public preset or an empty map — this is the field's own declaration, not a special rule for the map type.

### `null = missing`

Config uniformly stipulates for JSON objects:

> `"key": null` is exactly equivalent to the key not existing.

Before entering migration / reconcile, all object keys whose value is `null` are recursively removed; `null` inside arrays is not subject to this rule. Tool writes and saves also obey the same normalization and do not persist a second semantic of "explicit null". If a distinction between an explicit value and missing is needed, it must be modeled with an explicit enum.

### Reconcile Logic Chain

```text
object key:null → remove the key (treated as missing)

static object missing / type error
  → recurse downward, assembling the result of each static child

leaf missing / type error
  → that leaf's default

map field missing / type error
  → that map field's own default

map already present
  → do not merge keys from the map default back
  → iterate existing non-null entries
  → recursively reconcile each entry's fixed value schema

unknown path
  → recursively remove and report item by item
```

What is dynamic is the map's **key**, not the fields inside an entry.

Therefore:

- When the map node `providers` is missing, use `providers`'s own default;
- when a key in an existing map is missing or null, that key does not exist and is not created;
- when a map already exists, keys that do not appear in the default map are not automatically merged back;
- when the value of an existing key has a type error, that value object is defaulted directly according to its fixed child defaults;
- a single child's problem only falls back to that child, and must not take down the entire parent object because of it.

## Reflection and Consumer Projections

The complete descriptor tree must keep locatable nodes for **all containers**: static objects, maps, and the fixed object values of existing map entries are all in the tree. Therefore `query(path, view?)` can uniformly query any visible container or leaf; the local complete tree, for example, has `kaomoji`, `kaomoji.system.idle`, `kaomoji.system.idle.face`, while the LLM restricted projection only exposes the accessible `kaomoji.user` subtree. Whether the local UI renders the object itself as an independent node is a separate flat-presentation choice and does not change the descriptor tree.

`reflect()` is responsible for projecting the type, doc comment, constraints, and current value of `Config` into nodes; the local CLI and settings panel consume the complete projection. For example, the local complete projection may include:

```json
{ "path": "llm.active", "type": "enum", "options": ["unconfigured", "debug", "deepseek"],
  "value": "deepseek", "desc": "当前 LLM" }
```

Convention: doc comment → `desc`, `#[schemars(range(...))]` → min/max, serde default → initial value; the mechanical mapping from types to controls is bool→toggle, enum→radio group, int+range→slider, int→number box, string→text box, map→key-value list, nested object→grouped by path prefix.

**Two manual hooks** remain the only non-automatic points:

1. **Dynamic enum options**: the type system cannot express that the legal values of `llm.active` are `unconfigured`, `debug`, and the keys of `providers`; a single `OPTIONS` registry provides `path → fn(&Config) -> Vec<String>`.
2. **Hot/cold semantics**: reported truthfully by the runtime diff; the classification of concrete fields is only defined in their behavior docs, and items not listed as hot fields are cold updates by default — written to disk but the current running value is kept. Hot updates take effect from the next runtime operation onward: they do not change an already-sent LLM request or an in-flight tool call; subsequent runtime operations read the new value. Cold updates uniformly take effect after the whole application / backend process restarts. When the agent itself updates, a hot field's tool result explains "took effect" in `msg`, and a cold field explains "saved, takes effect after restarting the app" in `msg`; when the user modifies via the reflected Config UI, only the corresponding UI field shows a `restartRequired` status, and no event, chat, or system message is proactively injected to the agent. When the agent later queries a concrete value that has a pending restart change, the query result explains "saved, takes effect after restarting the app" in `msg`.

`no_llm_visible` forms another **LLM restricted projection** on the descriptor tree. It is not path hardcoding: any marked node and its descendants are removed from the LLM projection, and direct access is uniformly denied.

The current `llm` subtree should be marked `no_llm_visible` in its entirety: including `llm.active` and `llm.providers`. Local configuration may still use private providers on the machine; these endpoints, models, environment variable names, and the active selector are not exposed to the LLM and cannot be modified by the LLM.

## `edit_config`: Single Tool, Explicit Action, Progressive Disclosure

The LLM has only one `edit_config` tool. Behavior is expressed by the required `action` branch; switching modes via missing parameters, null values, or failed writes is forbidden:

| action | Input | Semantics |
|---|---|---|
| `grep` | `pattern` | use Rust regex to locate candidates in the paths and Chinese `desc` of LLM-visible nodes |
| `query` | exact `path`, optional `view` | read the value, type, description, and one-level structure of the specified node |
| `update` | exact `path` + JSON `value` | modify config through the unified validation, hot-apply, and persistence pipeline |

The tool description gives a one-line Chinese purpose for each **LLM-visible top-level key**; it does not provide a root query branch. Currently included:

- `kaomoji`: kaomoji state mapping;
- `set_autonomy_default_ttl_ms`: Autonomy default duration;
- `timer`: Timer scheduling subtree;
- `view_scale`, `badge_style`, `badge_side`: View appearance;
- `theme`, `themes`: current theme name and theme token table (docs/theme.md).

The recommended steps keep only one sentence: **when the path is unknown, `grep` first, then `query`; prefer `view=children`, and use `view=object` only when necessary for an already-located small object.**

### `grep`

`grep.pattern` is a Rust regex matching path and Chinese `desc`, using the original default semantics: case-sensitive by default and Unicode enabled; when case-insensitive matching is needed, explicitly use an inline flag (such as `(?i)timer`); no lower-casing, word segmentation, fuzzy matching, or keyword expansion is done.

A legal regex with no match returns a successful empty array `{"ok": true, "matches": []}`; only invalid regex syntax returns an error. Candidates may hit leaves and containers, returning `path + type + desc` and **not returning the current value**; results are stably sorted by full `path` lexicographic order. After the exact path is obtained, `query` reads the true value.

### `query`

`query` always queries **one node** by exact path and uniformly returns `node + children`; it is not three actions leaf / children / object.

- If the path is a leaf: `node` carries `path / type / desc / value`, and `children: []`;
- if the path is a container and `view` is omitted or `view=children`: returns the target `node` and direct `children`; leaf children carry the current value, container children do not carry values recursively;
- if the path is a container and `view=object`: returns the complete current JSON of the target object / map; recommended only for already-located small objects, and it does not return `children`;
- `view=object` only applies to containers. If it is passed for a leaf, it explicitly errors and does not silently change to another read shape.

Only a `query(path)` carrying the complete current value of the update target leaves a read snapshot of that exact path: a leaf's direct `query(path)` is complete; an object / map must use `query(path, view="object")`. `grep` and the container default `view=children` are only for discovery/navigation and produce no snapshot. Each snapshot is associated with its own tool result message ID (the existing `tool_call_id`), and the source is not inferred from summary text.

Whether a snapshot can be used for an update is a set-coverage judgment. Let `R` be the snapshots produced by all complete queries, `C` the set of message IDs still retained in Context, and `W` the successful writes after the snapshot; a target `P` is writable if and only if there exists `r ∈ R` such that:

```text
r.path = P
r.toolResultMessageId ∈ C
r comes from an earlier LLM response
there is no successful write that intersects P and occurred after r
```

Therefore the same path may have multiple snapshots; after an earlier snapshot is invalidated by a write, a later complete query can provide valid coverage again. Context compression does not guess invalidation entry by entry; it takes `R ∩ C`: snapshots corresponding to query results still in Context remain valid, and snapshots not covered by the set retained in Context naturally become invalid and can be cleaned up. A successful write only pollutes the snapshot set of intersecting paths and does not affect unrelated paths.

`update(path, value)` must have the valid complete snapshot described above; otherwise it is atomically rejected and requires a fresh query. The agent does not carry a revision or compare-and-swap parameter.

### Response Size Guardrail

For the **aggregate result** of all actions, measured as the final serialized tool result UTF-8 JSON byte count, the maximum is **1 KiB**.

- When a `grep` regex is too broad, `query(children)` has too many children, or `query(object)` has an oversized object, explicitly reject;
- no truncation, no automatic pattern narrowing, no automatic view switching; the error must state the actual size, the limit, and the direction to narrow down next;
- the exact query of a **single leaf** is the exception: it is returned in full regardless of the value's own size and must not be truncated or have its view changed.

### `update` and null

`update` reuses the unified modification pipeline of the local CLI / panel: write → null normalization → reconcile / serde structural validation → run the validators of the target node's subtree, then the validators of all ancestor nodes → persist the candidate → hot-apply and replace the live Config → truthfully return `restartRequired`. Unrelated branches are not revalidated; if any validator of the candidate Config fails, the entire update does not take effect and both the in-memory and on-disk Config remain unchanged. A candidate persistence failure also does not change the live Config; persistence must be atomic, so that disk can only hold the old complete file or the new complete file. Validation runs in subtree→ancestor order; errors are aggregated stably by full path lexicographic order and returned.

`value: null` is only allowed for a leaf, making the leaf return to its own default per the `null = missing` rule. object / map nodes reject a `null` update, to avoid implicitly turning `null` into deletion of a dynamic map entry. Config provides no separate `add` / `delete` actions.

## Unified Modification Entry Point

All config writes must share the same validation and application pipeline:

| Entry point | Form |
|---|---|
| LLM `edit_config` | `grep / query / update` filtered by `no_llm_visible` |
| `ambery-cli` | `list` / `get <path>` / `set <path> <value>` / `schema`; by default over HTTP for hot effect and broadcast, with `--offline` writing files directly as fallback; zero per-field subcommands |
| Settings panel | the 4th webview window; opened by right-clicking the tray, closes on focus loss; a mechanical renderer of the complete schema, with "Show/Hide" and "Quit" at the bottom |

Server API: `GET /config/schema` returns the node list, `readOnly`, and version; `POST /config {path, value}` performs validation, hot-apply, persist, `config_changed` broadcast, and returns `restartRequired`; there is also the existing `GET /config` frontend runtime view (kaomoji, viewScale, etc.), which is not changed in this design. The three modification entry points share this unified pipeline; validation must exist only once.

The unified pipeline is responsible for default/null normalization, recursive reconcile, serde validation, validator execution, dynamic enum validation, hot-apply, persistence, and `restartRequired` reporting.

### External File Auto-Load

External modifications to `config.json` are monitored at runtime and automatically loaded.

- **File moved or deleted**: keep the current live Config unchanged and show "Config file moved or deleted" in the reflected Config UI; do not auto-rebuild the default file or write back; automatically retry when the file is detected to reappear.
- **Read, parse, or candidate validation failure**: keep the current live Config unchanged and show the concrete load error in the reflected Config UI; automatically retry on later file changes until a repaired file is detected.
- **Legal read and candidate passed**: exactly the same as a full-text update: migration → null normalization → reconcile → serde structural validation → all validators → diff against the live Config. External loads cannot bypass validation, atomicity, or hot/cold effect boundaries; any pending-state changes for cold fields are recalculated from the difference between saved and running values.
- **Apply diff**: hot fields are explicitly applied immediately; cold fields keep the current running value and show `restartRequired` status in the UI; all agent read snapshots intersecting actually changed paths are marked dirty.

### Pending Restart State

Pending restart state equals the saved config value being different from the current running value; when the two become the same again, the state clears immediately.

### Startup Load

Loading has no single update target, so all validators run; all errors are aggregated once by full path lexicographic order and written to the load report, but startup is not blocked. Each erroneous node is defaulted per the established failure semantics, and startup then proceeds with the repaired Config.

## Principles

- **Current structure and field semantics colocated**: a Config field is the declaration source for type, description, default, migration metadata, and consumer access metadata.
- **Scope of this document**: this document only explains Config's general mechanisms: persistence, version/migration, default, null, validation, reflection, access projection, and the unified modification pipeline; business behavior triggered by concrete fields and tool alternation flows are defined in their respective behavior documents.
- **Version range determines migration**: explicit `Default / Rename / Func / RenameWithFunc` handles historical intervals that deviate from same-path preservation; only a miss is the unique, explicit implicit `Current`.
- **Parent-child rules de-conflict by version**: parent and child may each have metadata; only intersecting explicit source-version ranges are rejected, avoiding execution-order and coverage guessing.
- **Failures default locally**: any migration or validation failure only defaults the sick node, reports item by item, and continues; one child problem must not take down the entire Config.
- **null has no second semantic**: an object's `key:null` is equivalent to missing; use an explicit enum when a third state is needed.
- **Objects construct downward, leaves and maps carry their own default**: a static object's default shape comes from its children; map defaults do not merge dynamic keys back just because the map already exists.
- **No special rules; use semantics to make behavior explicit**: `edit_config` behavior is fully expressed by the schema's `action` branches; do not silently switch semantics via missing parameters, null values, or failed writes.
- **Progressive disclosure, query as needed**: the LLM first greps to locate, then queries to read the exact current structure and value, and finally updates; walk the hierarchy as needed rather than guessing paths or injecting the complete schema.
- **Prefer modifiable, restrict exceptions**: Config allows agent modification by default; mark `no_llm_visible` only when (1) it has major impact on the agent, or (2) it is irreversible and easy to break.
- **Access projection does not change truth**: `no_llm_visible` only restricts the read/write projection of the LLM tool; it does not change local Config, persistence, or local management capabilities.
- **Report truthfully**: hot application takes effect immediately; restart requirements, migration fallback, unknown-path removal, and response-size rejections must all be explicitly returned and auditable.
- **Single lock, single truth**: all Config entry points and external auto-load are serialized through the same lock; only a complete old state or a complete new state is observable; no entry-point-private drafts are kept, eliminating visible state forks and read/write races.
