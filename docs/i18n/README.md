# Ambery Documentation i18n

English | [中文](README.zh.md)

Bilingual pairing rules for `docs/`, [concepts.md](../../concepts.md), [spec.md](../../spec.md), [docs-spec.md](../../docs-spec.md), `CONTRIBUTING.md`, and `README`.

- `foo.md` is the English document; `foo.zh.md` is the Chinese document. Both sides carry equal authority.
- Keep prose in one language per file; do not mix paragraphs. Machine contract strings, paths, commands, and code fences are language-neutral.
- Use the fixed term table in [glossary.md](glossary.md).
- Agent instruction files (`AGENTS.md`) are English-only and are exempt from pairing.
- After editing either side, update the other side and run `node scripts/verify-i18n-pairs.mjs`.
