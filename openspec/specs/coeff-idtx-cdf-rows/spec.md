# coeff-idtx-cdf-rows Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-idtx-cdf-rows`.

## Requirements
### Requirement: FSC and IDTX coefficient CDF rows

The decoder tile CDF subset SHALL expose crate-private `TileCoeffBaseBobCdf`,
`TileCoeffBaseIdtxCdf`, `TileCoeffBrIdtxCdf`, and `TileIdtxSignCdf` rows for the
AV2 §8.3.2 FSC/IDTX coefficient selectors, tracked by
`DECODE-COEFF-IDTX-CDF-ROWS`. The row boundary SHALL load generated AV2 §9.3
defaults, SHALL validate q-context, pre-clamped `tx_size_ctx`, and row-specific
symbol-context axes with typed `TileCdfError::SelectorOutOfRange` errors, SHALL
participate in tile copy/save/average and frame-end count scaling, and SHALL NOT
claim runtime `useFsc` coefficient decode support.

#### Scenario: Generated FSC and IDTX rows are selectable

- **WHEN** a tile CDF subset is copied from frame defaults
- **THEN** `CoeffCdfSelector::BaseBob`, `BaseIdtx`, `BrIdtx`, and `IdtxSign`
  return rows matching the generated AV2 §9.3 defaults for valid q-context,
  pre-clamped transform-size context, and symbol-context axes
- **AND** the rows are available through immutable and mutable row access

#### Scenario: Invalid FSC and IDTX axes are rejected

- **WHEN** a FSC/IDTX coefficient selector supplies an out-of-range q-context,
  `tx_size_ctx`, or symbol context
- **THEN** row selection returns a typed selector error naming the specific tile
  CDF array and offending axis
- **AND** no symbol decoder state is consumed by a failed row selection

#### Scenario: FSC and IDTX rows use the tile CDF lifecycle

- **WHEN** tile CDF rows are copied, saved, averaged, or scaled for frame-end
  update in the supported subset lifecycle
- **THEN** `TileCoeffBaseBobCdf`, `TileCoeffBaseIdtxCdf`,
  `TileCoeffBrIdtxCdf`, and `TileIdtxSignCdf` rows participate in the same
  lifecycle behavior as the other loaded coefficient rows

#### Scenario: Runtime FSC decode remains unchanged

- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call a FSC/IDTX coefficient symbol pass yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into these selectors
