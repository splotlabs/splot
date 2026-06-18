## Why

The ordinary coefficient branch now derives generated `Tx_Width[txSz]`,
`Tx_Height[txSz]`, `Tx_Width_Log2[txSz]`, and `Tx_Height_Log2[txSz]`, but the
remaining transform-size tables needed for the next coefficient wrappers are
still skipped by `cargo xtask gen-tables` because they contain `TX_*` symbols.
Those tables are normative AV2 section 9.2 tables in `all_tables.h`; per the
decoder mission rules they should be generated, not copied into decode code by
hand.

## What Changes

- Add Feature ID `DECODE-TX-SIZE-SYMBOLIC-TABLES`.
- Extend `cargo xtask gen-tables` with a narrow AV2 `TxSize` symbol resolver,
  grounded in AV2 section 6.19.6.1, for `Adjusted_Tx_Size`,
  `Tx_Size_Sqr`, and `Tx_Size_Sqr_Up`.
- Remove those three tables from the symbolic-table skip allowlist and
  regenerate `splot-core::tables::conversion`.
- Add focused generator and generated-table spot tests proving the enum ordinal
  mapping and representative table values.
- Update decoder tracking, roadmap, generated status/coverage docs, and this
  OpenSpec change.

## Capabilities

### New Capabilities
- `tx-size-symbolic-tables`: Generated AV2 section 9.2 TxSize enum-valued
  conversion tables are available from `splot-core::tables::conversion`.

### Modified Capabilities
- `decoder-support`: Record the generated TxSize symbolic-table support row and
  proof.

## Impact

- Affected code: `xtask/src/gen_tables.rs`,
  `xtask/src/gen_tables/block_symbols.rs`,
  `crates/splot-core/src/tables/conversion.rs`, and table spot tests.
- Affected docs/tracking: implementation matrix, decoder support matrix,
  decoder conformance coverage, decoder roadmap, generated status docs, and
  this OpenSpec change.
- No public API, dependency graph, CLI behavior, output behavior, runtime
  `coeffs()` wiring, `txSzCtx` derivation, `compute_tx_type`, scan derivation,
  dequantization, reconstruction, or broad `decode_block()` / `decode_tile()`
  support is added.
