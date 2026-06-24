# Agent Documentation

This directory holds progressive-disclosure guidance for agents and contributors.
Keep `AGENTS.md` as the high-level entry point; place task-specific details here
instead of growing the root file.

## Logical Grouping

| Topic | File |
|---|---|
| Workspace ownership, dependency direction, concurrency, zero-copy | [architecture.md](./architecture.md) |
| Unconditional engineering judgment and "lazy senior developer" rules | [`AGENTS.md` § 0](../../AGENTS.md#0-unconditional-agent-behavior) |
| Day-to-day contribution workflow, Feature IDs, OpenSpec, commits | [workflow.md](./workflow.md) |
| Local commands, CI gates, generated-doc commands, fuzzing | [commands.md](./commands.md) |
| Rust/library conventions, panic policy, docs, SPDX, source size | [coding-standards.md](./coding-standards.md) |
| AV2 spec source of truth, citations, diagnostics | [av2-spec-and-diagnostics.md](./av2-spec-and-diagnostics.md) |
| Parser, fuzz, conformance, and proof expectations | [testing.md](./testing.md) |
| Encoder research gate and third-party codec references | [encoder-reference-gate.md](./encoder-reference-gate.md) |
| Documentation and AV2 conformance audit workflows | [audits.md](./audits.md) |
| Licensing and third-party material boundaries | [licensing.md](./licensing.md) |

## Refactor Notes

`AGENTS.md` is canonical for dependency direction. If another summary disagrees
with it, update that summary rather than changing the crate graph by implication.

Flagged for deletion: none. The original root instructions are repo-specific
enough to keep; this refactor relocates details instead of discarding them.
