# Spec — packages/agents/opencode

English | [中文](spec.zh.md)

## Technology choices

- **Filter module** (`filter/opencode.rs` moves here): noise list, block splitting, and line-wrap merging parameters to be converged after real samples.

## Architecture decisions

1. **Hook shape deferred**: opencode's push channel differs from Claude Code's; the shape is decided from real opencode behavior, not assumed from the claude package.

## Fixed constraints

- Same contract as every agent package: register via `ambery-terminal-lib`, three-state read, no content in hook payloads.
