# Spec — packages/terminal-lib

English | [中文](spec.zh.md)

## Technology choices

- Rust library crate (`ambery-terminal-lib`), no binary, no runtime of its own.
- Owns: the adapter trait (`enumerate` / tri-state `read`), envelope types (Source projection, three-state read Content / Gone / Error), composite dispatch (enumeration-routed), the platform-primitives trait (terminal-host environment capabilities), and the test stub (MapAdapter).

## Architecture decisions

1. **Contract crate first**: `terminals/*` and `agents/*` depend only on this crate; core depends only on this crate. A terminal package never depends on an agent package or on core.
2. **Trait signatures follow the Ambery Protocol**: the current trait shape is provisional; it is re-derived from the protocol contract when T35 (access protocol) lands — this crate freezes no trait before that.

## Fixed constraints

- Three-state read (Content / Gone / Error) is the only read contract — no partial reads, no guessed states.
- Interruptive host actions (desktop switching) always pass explicit consent; a terminal package never switches on its own.
