# Tasks: conformance-negative-mutator

## 1. Mutator harness

- [x] 1.1 A committed harness (a `#[test]`, sharing the corpus seeds) with a
  table of `(seed vector, mutation, expected rule_id, spec citation)` rows. Each
  reads a committed valid seed, applies a documented deterministic mutation in
  memory, validates with `splot-validate`, and asserts the expected error
  `rule_id` is present and the validator did not panic.
- [x] 1.2 Target stable, decidable diagnostics only (IVF container, OBU header,
  LEB128 framing). Every expected `rule_id` MUST be verified against the
  diagnostic registry (run the mutation and confirm the emitted id) — never
  invent one.

## 2. Docs + matrix

- [x] 2.1 `CONF-AVM-INVALID-STREAMS`: status honest with proof (the harness test,
  the exercised diagnostics); set `openspec_change`. Update `docs/CONFORMANCE.md`
  to describe the negative mutator.

## 3. Verification

- [x] 3.1 Each mutation row fires its expected diagnostic; a no-panic assertion
  holds. The set is non-vacuous (≥3 distinct diagnostics across container /
  header / framing).
- [x] 3.2 `cargo xtask ci` (bare, exit checked) passes.
