# Documentation Hierarchy

This file defines the authority hierarchy for ai-wiki design documents.

## Authoritative (current)

These documents define the system as it exists or is being built:

| Document | Role |
|----------|------|
| `../README.md` | User-facing summary — install, usage, examples |
| `superpowers/specs/2026-04-10-multi-wiki-design.md` | **Canonical spec** — multi-wiki config, CLI resolution, MCP changes |
| `design/application.md` | Architecture reference — crate structure, CLI subcommands, MCP tools |
| `design/workflow.md` | Workflow reference — ingest/process phases, wiki format |
| `design/prompt.md` | Concept document — the "LLM Wiki" pattern (from Tobi Lutke's gist) |

When documents conflict, **the multi-wiki spec takes precedence** over application.md and workflow.md.
The README is a user-facing summary derived from the spec and architecture docs.

## Historical (non-normative)

These documents are preserved for context but do not define current requirements:

| Document | Role |
|----------|------|
| `superpowers/specs/2026-04-09-ai-wiki-design.md` | Original single-wiki spec — superseded by multi-wiki design |
| `superpowers/reviews/2026-04-09-code-review-findings.md` | Code review findings — snapshot of issues found at that date, not open requirements |
| `superpowers/plans/2026-04-09-ai-wiki-implementation.md` | Implementation plan — execution checklist, not a design authority |
| `archive/*.txt` | Raw Claude session transcripts — brainstorming records, not specs |
