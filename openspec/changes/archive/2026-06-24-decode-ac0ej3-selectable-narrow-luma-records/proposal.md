## Why

The live ac0ej3 decode probe now advances past active CfL mode syntax and stops
at the first luma-only `BLOCK_4X32` SDP leaf while deriving `TX_MODE_SELECT`
`LrTxSkip` transform records. This is a narrow stale admission gate in the
selectable transform-record path: the leaf has valid luma syntax but no chroma
syntax, and the decoder must consume it before it can reach decoded sample
population.

## What Changes

- Track `DECODE-AC0EJ3-SELECTABLE-NARROW-LUMA-RECORDS` as the next partial
  ac0ej3 Wiener NS LR prerequisite.
- Admit and parse luma-only narrow `TX_MODE_SELECT` leaves needed by the local
  stream, starting with the observed `BLOCK_4X32` luma SDP case.
- Derive luma transform records and `LrTxSkip` values for those leaves without
  fabricating chroma records.
- Keep the runtime fail-closed before decoded `CurrFrame`/`CdefFrame` sample
  population, `FilterClass` retention, loop-restoration filtering/output,
  reference refresh, or AVM/dav2d byte equality.

## Capabilities

### New Capabilities

- `ac0ej3-selectable-narrow-luma-records`: fail-closed parsing of the ac0ej3
  luma-only narrow selectable transform-record subcase.

### Modified Capabilities

- `ac0ej3-selectable-transform-records`: record that the narrow luma-only
  subcase is a distinct prerequisite within the broader selectable
  transform-record frontier.
- `decoder-support`: add a support row for the new partial ac0ej3 prerequisite.

## Impact

- Affects `crates/splot-decode` runtime-private Wiener NS LR transform-record
  handoff code and focused tests.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, and generated status documents.
- Does not add dependencies, public API surface, external oracle invocation,
  encoder behavior, broad AV2 conformance claims, or successful ac0ej3 output.
