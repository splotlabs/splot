# Change: conformance-negative-mutator

## Feature IDs

- `CONF-AVM-INVALID-STREAMS`

## Why

The conformance corpus (the archived `conformance-corpus-foundation` change)
proves that valid vectors validate clean and has a single bootstrap negative (a
truncated IVF). It does not yet systematically prove that *malformed* streams
produce the *expected* diagnostics without panicking — the
`CONF-AVM-INVALID-STREAMS` guarantee.

This change adds a committed, deterministic **negative mutator**: a small table
of targeted mutations applied to the committed valid seed vectors, each asserting
the specific diagnostic `rule_id` the validator must emit (and that it never
panics). Unlike the fuzz targets (random, no-panic only), these are *targeted*
malformations with a *named expected diagnostic*, so a regression that stops
emitting a diagnostic — or emits the wrong one — fails CI.

## Scope

- A committed mutation harness that, for each `(seed, mutation, expected
  diagnostic)` row, reads a committed valid vector, applies a documented,
  deterministic byte/field mutation in memory, runs `splot-validate`, and
  asserts the expected error `rule_id` is present (no panic). Targets stable,
  decidable diagnostics (IVF container, OBU header, LEB128 framing).
- Each mutation cites the spec section / rule it exercises and uses an
  **actually-registered** `rule_id` (verified against the diagnostic registry,
  never invented).
- Runs in CI (a `#[test]`), no AVM, no network.

## Non-goals

- No random fuzzing (the cargo-fuzz targets own no-panic over arbitrary bytes).
- No deep frame-header bit-surgery whose exact diagnostic is fragile; target
  stable container/header/framing diagnostics.
- No new validator diagnostics; this exercises existing ones.

## Acceptance criteria

- [ ] A committed CI test applies each targeted mutation to a committed seed and
  asserts the expected registered diagnostic with no panic; each row cites its
  rule. `CONF-AVM-INVALID-STREAMS` records proof. `cargo xtask ci` green.
