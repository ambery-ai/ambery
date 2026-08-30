# Spec — packages/terminals/wt

English | [中文](spec.zh.md)

## Technology choices

- **C# (.NET) sidecar** (`ambery-uia-sidecar`): UIA enumeration, tab location, TermControl full-text read.
- Stays C#: the wt reading path was verified end-to-end in C#; rewriting it in Rust would re-buy that verification for no user-visible gain. The official MCP SDK exists for C#, so protocol migration needs no rewrite.
- Packaging: self-contained win-x64 (not single-file), zero .NET runtime dependency for users; distributed as a Tauri externalBin.

## Architecture decisions

1. **Separate process**: the sidecar is its own exe, lazily spawned on first request; dead is discarded, the next request starts it fresh. Crash isolation is the point.
2. **stdio JSONL protocol**: request/response lines; the command set (list_windows / read_tab / switch_tab) is this package's private contract with the host until the Ambery Protocol supersedes it.
3. **Visibility before switching**: WT's "show windows on all desktops" makes everything UIA-readable without desktop switches; `vd_switch` is the explicit-consent fallback, never automatic.

## Fixed constraints

- Windows-only package; on other platforms the host's Option chain degrades naturally (sidecar absent = read channel falls back to Context).
