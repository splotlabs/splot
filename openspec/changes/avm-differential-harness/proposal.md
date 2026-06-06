# Change: avm-differential-harness

## Feature IDs

- `CONF-AVM-DIFF-HARNESS`

## Why

AVM is the reference software and our conformance oracle. A differential harness
lets us compare `splot` against AVM systematically rather than by hand.

## Scope

- Crates/modules: `xtask` (the `conformance` task).
- Direction: first `avm encode` → `splot validate`; later `splot encode` →
  `avm decode`.

## Non-goals

- No vendoring of AVM or of unclear-license vectors.
- No network access in normal CI.

## Acceptance criteria

- [ ] `cargo xtask conformance` runs a documented `avm encode` → `splot validate`
      comparison against a local AVM checkout/corpus.
- [ ] Results are reproducible from a documented command.
- [ ] Proof is recorded in the `CONF-AVM-DIFF-HARNESS` row (`avm_diff` stage).

> Status: **proposed**. Not implemented (the `conformance` task is a stub).
