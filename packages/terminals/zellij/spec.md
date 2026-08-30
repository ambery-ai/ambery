# Spec — packages/terminals/zellij

English | [中文](spec.zh.md)

## Technology choices

- **In-process Rust CLI calls** (`zellij action`): pane listing, screen dump — the CLI itself is the transport; no sidecar process.

## Architecture decisions

1. **No separate process**: unlike wt, the CLI's own lifetime is the isolation boundary; a sidecar would add a hop with no gain.
2. **Cross-platform**: works wherever zellij runs (macOS/Linux); no UIA, no platform gate.

## Fixed constraints

- Pane title must carry the marker for locate to hit (title convention owned by the agents/claude package); the locate query strategy lives in the access pipeline, not hardcoded in this package.
