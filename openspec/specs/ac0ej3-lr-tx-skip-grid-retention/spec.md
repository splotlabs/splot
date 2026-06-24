## Purpose

Track the ac0ej3 fail-closed runtime prerequisite for retaining value-backed
`LrTxSkip` grid values before live tile traversal wires transform records into
loop-restoration classification.

## Requirements

### Requirement: ac0ej3 LR Tx-Skip Grid Retention

The decoder SHALL track `DECODE-AC0EJ3-LR-TX-SKIP-GRID-RETENTION` as a partial
ac0ej3 Wiener NS LR prerequisite. The helper SHALL derive luma `LrTxSkip`
storage values from caller-provided transform records using the AV2 §5.20.7.24
rule `skip_flag || (eob == 0)`, SHALL retain them in the bounded
`WienerNsLrTxSkipGrid` representation read by §7.20.4 classified-Wiener
classification, and SHALL reject incomplete or out-of-bounds grids instead of
fabricating missing values.

#### Scenario: Transform records populate a complete grid

- **WHEN** caller-provided luma transform records cover every retained 4x4 grid
  cell
- **THEN** the helper returns a `WienerNsLrTxSkipGrid` whose lookups expose the
  parsed `skip_flag || (eob == 0)` values
- **AND** no default or sentinel value is used for any returned grid cell

#### Scenario: Missing cells are rejected

- **WHEN** caller-provided luma transform records leave one or more retained
  4x4 grid cells without a parsed transform source
- **THEN** the helper returns a structured reconstruction error
- **AND** it does not return a partially fabricated `WienerNsLrTxSkipGrid`

#### Scenario: Out-of-range records are rejected

- **WHEN** a caller-provided transform record extends outside the retained
  `LrTxSkip` grid dimensions
- **THEN** the helper returns a structured reconstruction error
- **AND** it does not mutate a returned grid with truncated or clamped values

#### Scenario: Live ac0ej3 remains fail-closed

- **WHEN** the local ac0ej3 mission stream reaches the live LR runtime storage
  retention boundary
- **THEN** the runtime still returns `decode/unsupported-feature`
- **AND** it does not claim live decoded samples, live tile-populated
  `LrTxSkip` values, LR filtering, output, reference refresh, AVM/dav2d byte
  equality, or successful ac0ej3 decode
