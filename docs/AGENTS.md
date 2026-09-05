# AGENTS.md — Documentation standards

English | [中文](AGENTS.zh.md)

> Responsibility boundaries (which document says what) belong to `docs-spec.md`; this file only governs "how to write". Before changing any document, read this file and `docs-spec.md` first.

## Document structure

- A document's topic and tree position fix its scope: describe its own topic with appropriate detail; for children, describe only purpose, responsibility, and high-level behavior, and link details to their owner.
- Document type does not broaden scope; a referenced document is only exhaustive about its own topic. Test mechanisms, fixtures, and harness belong to the lowest ownership layer; higher-level documents only link.

## Responsibility layers

one home per fact — a topic is fully stated in only one document; when mentioned elsewhere, only link and do not repeat content.

| Layer | Responsibility | Not here |
|---|---|---|
| `docs-spec.md` | Navigation: responsibility map and admission boundaries for `docs/*.md` and `concepts.md` | Writing rules (→ this file), document body |
| `docs/*.md` | Contract body: one topic per file (defined by the docs-spec responsibility map) | Other documents' topics (→ link) |
| `concepts.md` | Concept model: domain terms and layering | Implementation mechanisms (→ `docs/`) |
| `spec.md` | Technology choices, structural decisions, tradeoffs | Runtime mechanisms (→ `docs/`), concept definitions (→ `concepts.md`) |
| `reports/` | Research conclusions: evidence + conclusion | Process records, contract body |
| Root `README.md` | Project entry: what it is, quickstart, platform matrix | Architecture details (→ `docs/`) |
| Root `VISION.md` | Project vision: one handwritten page | Design details (→ `docs/`) |
| This file | Repo-wide documentation writing standard | Responsibility map (→ `docs-spec.md`) |

Placement rules: undecided work → development tickets / `docs/post-0.1.0.md`; research → `reports/`; contracts → `docs/`; concepts → `concepts.md`; technology choices, structural decisions, tradeoffs → `spec.md`; entry → `README.md`.

The public document set = all rows of the table above (README / VISION to be created). `drafts/`, `user-goals.md`, `ideas.md`, `debug-lessons.md`, and `mem.md` are not public and do not enter the public repo.

## Writing rules

- Write the current state, not change history: avoid "previously/now/no longer", PRs, commits, and positional drift; name the current mechanism. Change stories go into commits / PRs.
- One paragraph per line (editor soft-wrap); code blocks, tables, and lists keep their formatting.
- Code comments write the complete contract, not a reasoning transcript: keep behavior, failures, timing, ownership, exceptions, consequences, and non-obvious choices; delete narration, test walkthroughs, review analysis, and code restatement.
- State facts directly and name the subject: write the concrete check, type, API, operation, or behavior; do not use metaphors (except defined terms).
- Example code must match the actual implementation and must not mislead.

## Slop list (hunt when writing docs)

- The same rule stated repeatedly in multiple documents: one home per fact, others link.
- History or war-story narration: "previously", "now", "no longer", "used to", "renamed", PR, commit.
- Implementation status markers ("implemented!", "future:"): status rots; code and repo layout carry status.
- Hand-copied catalogs / lists (tests, packages, status): forbidden when a source or generator is authoritative.
- Reasoning transcripts: step-by-step implementation narration, proofs of obvious branches, test walkthroughs, rejected local alternatives. Keep the final contract, delete the derivation path.
- Repeated rationale next to sibling entries: rationale is written once, at the owning capability / entry.
- Paragraph walls: a paragraph carrying multiple rules and parenthetical asides → split or demote to the owner.
- Emphasis inflation: bold / CAPS / "key" everywhere = no emphasis; reserve emphasis for behavior-changing clauses.

## Link rules

- Repo references always use relative Markdown path links; bare filenames are forbidden.
- Link targets must exist and anchors must not break; check references when modifying a linked document.

## Language rules

- Language direction = Chinese / English bilingual (the English version is implemented as a follow-up task); currently Chinese-first.
- No third-language mixing.
- Code identifiers, strings, and domain terms use English.

## Code-to-document reference rules

- Code files must not contain any document-name references (`docs/*.md`, `concepts`, `spec`, etc.); comments express meaning only.
- Document navigation is borne by `docs-spec.md`; code bears no links to documents.

## Mandatory process

- Before changing any document: first read `docs-spec.md` (responsibilities) and this file (standards).
- Commit document changes separately from code changes (atomic commit granularity, for review and rollback).
- Specific rules for README / VISION / CONTRIBUTING will be added when those are created.
