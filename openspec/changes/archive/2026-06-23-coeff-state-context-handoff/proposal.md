## Why

The ordinary non-FSC coefficient composer now commits final nonzero
`culLevel` and `dcCategory` values to tile context lines, but sign-source
derivation still receives `AboveDcContext` and `LeftDcContext` as caller slices.
The next runtime `coeffs()` bridge should read those DC context slices from the
same `TileCoeffContextState` it later mutates, preserving AV2 read-before-write
ordering without inventing extra caller-owned state.

Feature ID: `DECODE-COEFF-STATE-CONTEXT-HANDOFF`.

## What Changes

- Add a crate-private ordinary nonzero coefficient-pass handoff that reads the
  sign-source DC context slices from `TileCoeffContextState` before running the
  derived-base/derived-sign ordinary pass.
- Reuse the existing context-commit wrapper so the same state object is updated
  after the pass succeeds with the final `culLevel` and `dcCategory`.
- Preserve the existing lower-level APIs that still accept explicit DC context
  slices for focused tests and staged composition.
- Add tests for read-before-write ordering, pass failure preserving state, and
  invalid context-update facts preserving state.
- Update implementation/support matrices, decoder conformance coverage,
  roadmap notes, generated status docs, and OpenSpec artifacts.

## Capabilities

### New Capabilities

- `coeff-state-context-handoff`: ordinary non-FSC nonzero coefficient pass
  handoff that sources sign DC contexts from tile coefficient state and commits
  final context lines through that same state object.

### Modified Capabilities

- `decoder-support`: record the new partial decoder boundary and clarify that
  runtime `coeffs()` integration remains unsupported.

## Impact

Affected code is limited to crate-private `splot-decode` coefficient-loop
composition, tests, and tracking documents. There are no public API, dependency,
licensing, encoder, CLI, or fixture-output changes. The minimal runtime decode
path remains unchanged because real nonzero coefficient blocks still do not call
the ordinary coefficient-pass composer.
