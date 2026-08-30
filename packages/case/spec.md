# Spec — packages/case

English | [中文](spec.zh.md)

## Technology choices

- Rust binary crate (`ambery-case`), depends on `ambery-core` (case-runner feature) — the only package allowed to depend on core as a library consumer.
- Headless frontend testing embeds core + a TS test process via RemoteBridge, mirroring the Tauri shell shape.

## Architecture decisions

1. **Serves every package read-only**: case files may exercise core, terminal leaves, and apps; case does not expose its own contract surface to them.
2. **Snapshot is truth**: the case data section preserves JSONL raw text; replay derives assertions from the log, not from hand-written expectations.

## Fixed constraints

- No production behavior lives here; everything in this crate is test infrastructure.
