## ADDED Requirements

### Requirement: Tile coefficient state buffers

The decoder support model SHALL track `DECODE-TILE-COEFF-STATE-BUFFERS` as a
crate-private `splot-decode` row named `tile-coeff-state-buffers`. The row SHALL
cover decode-owned state for AV2 §5.20.7.27 transform-block-local `Level[]` and
`QuantSign[]` buffers and the tile-neighbour `AboveLevelContext`,
`LeftLevelContext`, `AboveDcContext`, and `LeftDcContext` lines read by §8.3.2
coefficient contexts. The row SHALL remain partial until the §5.20.7.27
`coeffs()` loop reads symbols, fills `Quant[]`, and wires reconstruction.

#### Scenario: Transform block buffers are bounded and initialized

- **WHEN** a transform-block coefficient state is constructed for caller-resolved
  adjusted dimensions
- **THEN** it allocates zeroed row-major `Level[]` and `QuantSign[]` arrays for at
  most the §5.20.7.27 32x32 adjusted block extent
- **AND** zero dimensions, dimensions above 32x32, arithmetic overflow, or
  allocation failure return typed errors rather than panicking

#### Scenario: Coefficient context lines update like coeffs

- **WHEN** a coefficient block completes with caller-supplied `culLevel`,
  `dcCategory`, `plane`, `x4`, `y4`, `w4`, and `h4`
- **THEN** the tile state writes `culLevel` to `AboveLevelContext[plane]` and
  `LeftLevelContext[plane]` over the block's above and left ranges
- **AND** it writes `dcCategory` to `AboveDcContext[plane]` and
  `LeftDcContext[plane]` over the same ranges
- **AND** out-of-range plane or coordinate facts return typed errors rather than
  panicking or silently wrapping

#### Scenario: Coefficient context lines reset like reset_block_context

- **WHEN** block syntax requests a level/DC context reset for caller-resolved
  plane, start, size, and subsampling facts
- **THEN** the tile state zeros the matching above and left level/DC context
  ranges
- **AND** the operation is bounded by the actual owned line lengths and cannot
  spin on pathological caller counts

#### Scenario: State does not change decode output yet

- **WHEN** the minimal flat-intra fixture is decoded to hash, raw, or Y4M output
- **THEN** output bytes remain unchanged because this change does not wire the
  §5.20.7.27 `coeffs()` symbol loop or reconstruction

#### Scenario: Broader coefficient decode remains incomplete

- **WHEN** decoder support and conformance coverage are generated
- **THEN** `tile-coeff-state-buffers` appears as a partial row linked to
  `DECODE-TILE-COEFF-STATE-BUFFERS`
- **AND** `tile-payload-decode`, `tile-cdf-selection-boundary`, reconstruction,
  and full decoder conformance remain partial
