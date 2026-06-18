## ADDED Requirements

### Requirement: Decode coefficient base CDF rows
The decoder tile CDF subset SHALL expose crate-private, loaded-but-unread CDF
rows for ordinary non-IDTX coefficient base, base-EOB, and base-range symbol
families, tracked by `DECODE-COEFF-BASE-CDF-ROWS`. The row boundary SHALL load
the generated AV2 §9.3 defaults, SHALL validate every selector axis with typed
`TileCdfError::SelectorOutOfRange` errors, and SHALL NOT consume symbols, mutate
coefficient state, or claim runtime coefficient decode support.

#### Scenario: Generated default rows are selectable
- **WHEN** a tile CDF subset is copied from frame defaults
- **THEN** coefficient base, base-EOB, and base-range selectors return rows that
  match their generated §9.3 default banks for valid selector axes
- **AND** the rows are available through both immutable and mutable row access

#### Scenario: Invalid selector axes are rejected
- **WHEN** a coefficient base/base-EOB/base-range selector supplies an
  out-of-range quantization context, transform-size context, coefficient
  context, plane set, or low-frequency/base-range context
- **THEN** row selection returns a typed selector error naming the owning CDF
  array and the offending axis
- **AND** no symbol decoder state is consumed

#### Scenario: Tile copy and lifecycle include the rows
- **WHEN** tile CDF rows are copied, saved, averaged, or scaled for frame-end
  update in the supported subset lifecycle
- **THEN** the coefficient base/base-EOB/base-range rows participate in the same
  copy, average, and count-scaling behavior as the previously exposed block CDF
  rows

#### Scenario: Runtime coefficient decoding remains out of scope
- **WHEN** the minimal runtime decode path is exercised after this change
- **THEN** it does not read coefficient base/base-EOB/base-range symbols
- **AND** it does not write nonzero `Level[]`, `QuantSign[]`, or `Quant[]`
- **AND** fixture output remains unchanged
