## ADDED Requirements

### Requirement: Parity-hidden coefficient base CDF row

The decoder tile CDF subset SHALL expose crate-private `TileCoeffBasePhCdf`
rows for the ordinary non-FSC parity-hidden DC `coeff_base` selector, tracked by
`DECODE-COEFF-BASE-PH-CDF-ROW`. The row boundary SHALL load generated AV2 §9.3
defaults, SHALL validate `coeff_cdf_q_ctx` and parity-hidden base `ctx` axes with
typed `TileCdfError::SelectorOutOfRange` errors, SHALL participate in tile
copy/save/average and frame-end count scaling, and SHALL NOT claim runtime
nonzero coefficient decode support.

#### Scenario: Generated parity-hidden rows are selectable

- **WHEN** a tile CDF subset is copied from frame defaults
- **THEN** `CoeffCdfSelector::BasePh` returns rows matching
  `Default_Coeff_Base_Ph_Cdf` for valid q-context and parity-hidden context axes
- **AND** the rows are available through immutable and mutable row access

#### Scenario: Invalid parity-hidden axes are rejected

- **WHEN** a parity-hidden coefficient base selector supplies an out-of-range
  quantization context or parity-hidden coefficient context
- **THEN** row selection returns a typed selector error naming
  `TileCoeffBasePhCdf` and the offending axis
- **AND** no symbol decoder state is consumed by a failed row selection

#### Scenario: Parity-hidden rows use the tile CDF lifecycle

- **WHEN** tile CDF rows are copied, saved, averaged, or scaled for frame-end
  update in the supported subset lifecycle
- **THEN** `TileCoeffBasePhCdf` rows participate in the same lifecycle behavior
  as the other loaded coefficient base rows

### Requirement: Derived first pass consumes parity-hidden base row

The state-derived ordinary non-FSC base/level first-pass helper SHALL map
`CoeffBaseSelection::Ph` to `CoeffCdfSelector::BasePh` and SHALL read it through
the existing coefficient base symbol reader when hidden parity becomes active
before the final DC coefficient.

#### Scenario: Hidden parity reaches BasePh at DC

- **WHEN** an eob>=5 luma ordinary non-FSC first pass reads enough nonzero
  pre-DC coefficients to set `isHidden`
- **THEN** the final DC coefficient's derived `coeff_base` selector is
  `CoeffCdfSelector::BasePh`
- **AND** the helper consumes the selected row and writes the decoded DC
  `Level[]` value instead of returning an unsupported selector error

#### Scenario: Runtime decode remains unchanged

- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the state-derived first-pass helper yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the helper
