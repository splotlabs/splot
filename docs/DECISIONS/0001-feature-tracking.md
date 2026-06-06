# 0001: Feature tracking framework

## Status

Accepted

## Context

AV2 is too large for ad-hoc TODO comments or a GitHub-only board. We need a
canonical, machine-readable record of what is implemented and how far, and
automation that prevents that record from drifting away from the code — without
adopting a heavyweight, network-dependent, or GitHub-coupled system.

## Decision

Use:

- **OpenSpec** (`openspec/`) for change intent and acceptance criteria.
- **`docs/IMPLEMENTATION-MATRIX.toml`** as the canonical status (one `[[feature]]`
  row per stable Feature ID, ten per-stage statuses, recorded proof).
- **GitHub Issues/Projects** as the execution queue (not canonical truth).
- **Tests / conformance** as proof, referenced from each matrix row.
- **`xtask`** for enforcement and reporting: `cargo xtask feature-status`,
  `check-feature-status`, and `spec-coverage`, with `check-feature-status` wired
  into `cargo xtask ci` and CI.

## Consequences

- Every meaningful feature gets a stable ID that appears in code, diagnostics,
  tests, OpenSpec, docs, and PRs.
- Agents can work from IDs and acceptance criteria, and pick the next task from
  `spec-coverage`.
- CI prevents status drift (unknown ids, missing proof on `done`, stale
  `FEATURE-STATUS.md`).
- Matrix maintenance is required for every feature PR — a deliberate, small tax.

## Alternatives rejected

- **README checklist only** — not machine-checkable; drifts immediately.
- **GitHub Project only** — couples truth to a hosted board; not in the repo; not
  enforceable in CI.
- **Rust TODO comments only** — unstructured; no proof; no coverage view.
- **Percent-complete tracking** — meaningless for spec conformance; replaced by
  per-stage statuses with required proof.
