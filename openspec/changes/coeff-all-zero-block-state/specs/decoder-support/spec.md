## ADDED Requirements

### Requirement: Coeff all_zero block state handoff

The decoder support model SHALL track `DECODE-COEFF-ALL-ZERO-BLOCK-STATE` as a
crate-private `splot-decode` row named `coeff-all-zero-block-state`. The row
SHALL cover the §5.20.7.27 `all_zero == 1` coefficient-block state effects for
the currently traced minimal luma and V branches: zero coefficient state,
`eob == 0`, zero `culLevel` / `dcCategory`, and above/left context-line writes
through `TileCoeffContextState`. The row SHALL remain partial until the full
§5.20.7.27 `coeffs()` loop reads nonzero EOB and coefficient symbols, fills
nonzero `Quant[]`, and wires reconstruction.

#### Scenario: Transform block state includes Quant

- **WHEN** crate-private transform coefficient block state is initialized for a
  caller-resolved adjusted extent
- **THEN** the decoder allocates zeroed row-major `Level[]`, `QuantSign[]`, and
  `Quant[]` buffers with checked dimensions
- **AND** checked accessors reject out-of-range coordinates or positions without
  panicking

#### Scenario: All-zero block applies coefficient context writes

- **WHEN** the all-zero coefficient-block helper is applied for caller-resolved
  plane coordinates and 4x4 transform dimensions
- **THEN** it returns `eob == 0`, `culLevel == 0`, and `dcCategory == 0`
- **AND** it initializes zero `Level[]`, `QuantSign[]`, and `Quant[]` state
- **AND** it writes zero level/DC values to the covered above and left tile
  context ranges through `TileCoeffContextState`
- **AND** malformed ranges fail with typed coefficient-state errors before
  mutating context state

#### Scenario: Minimal trace writes all-zero state

- **WHEN** the minimal flat-intra block-symbol trace reads the existing luma
  `txb_skip` and V `v_txb_skip` symbols as all-zero
- **THEN** it applies the all-zero block state helper after each read
- **AND** the no-output-change symbol-frontier test remains unchanged

#### Scenario: Full coefficient decode remains incomplete

- **WHEN** decoder support and conformance coverage are generated
- **THEN** `coeff-all-zero-block-state` appears as a partial row linked to
  `DECODE-COEFF-ALL-ZERO-BLOCK-STATE`
- **AND** nonzero EOB decode, coefficient scan walk, coefficient base/br/sign
  reads, `read_quant`, dequantization, reconstruction, and full decoder
  conformance remain partial
