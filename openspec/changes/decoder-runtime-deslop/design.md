## Context

The minimal decoder runtime had long branch ladders, repeated local geometry
derivations, verbose unsupported-feature messages, and stale private comments.

## Goals / Non-Goals

**Goals:**

- Move repeated runtime facts into small shared helpers.
- Keep unsupported paths explicit and fail-closed.
- Lower only budgets proven by local gates.

**Non-Goals:**

- No new AV2 coverage or conformance claim.
- No public API or dependency graph changes.
- No standalone audit tooling.

## Decisions

Prefer internal helper modules over another layer of public abstraction. Keep
the helpers crate-private and prove behavior with focused runtime tests plus the
full repository gate.

Use measured gate output for ratchets: comment density moves to 262,
duplication moves to 6327, and source-line hard allowances stay empty.

## Risks / Trade-offs

- [Risk] Helper extraction can hide simple logic. -> Mitigation: keep helpers
  local to `runtime_minimal` and retain focused tests.
