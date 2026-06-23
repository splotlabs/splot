# tx-size-symbolic-tables Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `tx-size-symbolic-tables`.

## Requirements
### Requirement: Generate TxSize symbolic conversion tables
The generated-table automation SHALL resolve the AV2 `TxSize` enum tokens
defined by AV2 v1.0.0 section 6.19.6.1
(`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-6-1`) for the
section 9.2 conversion tables `Adjusted_Tx_Size`, `Tx_Size_Sqr`, and
`Tx_Size_Sqr_Up`, and SHALL emit those tables into
`splot-core::tables::conversion` from the committed `all_tables.h` attachment.
The automation SHALL NOT resolve unrelated symbolic tables or silently accept
unknown symbols.

#### Scenario: Supported TxSize tables are generated
- **WHEN** `cargo xtask gen-tables` processes `all_tables.h`
- **THEN** `Adjusted_Tx_Size`, `Tx_Size_Sqr`, and `Tx_Size_Sqr_Up` are emitted
  as generated numeric arrays in `crates/splot-core/src/tables/conversion.rs`
- **AND** `cargo xtask gen-tables --check` accepts the committed output

#### Scenario: TxSize ordinals match AV2 semantics
- **WHEN** the resolver sees `TX_4X4`, `TX_64X64`, `TX_4X64`, or `TX_64X4`
- **THEN** it resolves them to the AV2 section 6.19.6.1 ordinals 0, 4, 23, and
  24 respectively

#### Scenario: Unknown symbols fail loudly
- **WHEN** one of the supported TxSize tables contains a symbol outside the
  modeled `TxSize` enum
- **THEN** table generation fails instead of emitting a guessed value

#### Scenario: Runtime coefficient wiring stays deferred
- **WHEN** the generated tables become available
- **THEN** no runtime `coeffs()` path, `txSzCtx` derivation,
  `compute_tx_type`, scan derivation, dequantization, reconstruction, output, or
  reference refresh behavior changes in this feature
