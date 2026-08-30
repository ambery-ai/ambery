# Spec — packages/agents/claude

English | [中文](spec.zh.md)

## Technology choices

- **`"command"`-type hook scripts** (PowerShell on Windows / shell on macOS): read stdin JSON → output sessionTitle (marker anchor) → fire-and-forget POST to the host's local port. No interpreter dependency beyond what Claude Code's `"command"` type requires.
- **Filter module** (`filter/claude.rs` moves here): content rules derived from real Claude Code terminal samples.

## Architecture decisions

1. **Hook is this package's push channel**: five lifecycle events (SessionStart / UserPromptSubmit / Stop / SessionEnd / Notification); the remaining 30+ Claude Code events stay reserved.
2. **Identity = session_id**: sid8 (first 8 chars) is the instance identity; same name, different life — reopening a project is a new lifecycle. The marker prefix (`<project>·<sid8>`) is immutable; the descriptive part may evolve.
3. **Installation ships with this package**: the global hook config entry (~/.claude/settings.json) is shipped and installed by this package (platform-specific scripts); first-run installation flow is its responsibility.

## Fixed constraints

- The read channel is the single source of content (hook payloads carry no content) — this package never bypasses the host's read contract.
