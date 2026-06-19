## Why

`compute_tx_type()` is the next coefficient-loop dependency, but its intra chroma
fallback needs the AV2 §9.2 `Mode_To_Txfm` conversion table. The table is still
explicitly skipped by `cargo xtask gen-tables`, so the decoder would have to
keep caller-resolving a `PlaneTxType` or hand-transcribe the mapping.

## What Changes

- Add Feature ID `DECODE-MODE-TO-TXFM-SYMBOLIC-TABLE`.
- Extend the §9 table generator's narrow symbolic resolver to map AV2 `TxType`
  symbols and generate `Mode_To_Txfm` from the committed spec attachment.
- Regenerate `splot-core::tables::conversion` so `MODE_TO_TXFM` is available to
  future `compute_tx_type()` work.
- Add mirror-backed spot checks and generator tests for the resolved TxType
  symbols.
- Update matrix, decoder support/conformance coverage, and roadmap/status notes.
- No runtime decode output changes.

## Capabilities

### New Capabilities

- `mode-to-txfm-symbolic-table`: generated AV2 §9.2 `Mode_To_Txfm` table support
  for future transform-type computation.

### Modified Capabilities

- `decoder-support`: records the generated `Mode_To_Txfm` table as partial
  decoder infrastructure and keeps `compute_tx_type()` / runtime coefficient
  loop wiring as residual work.

## Impact

- Affected code: `xtask/src/gen_tables/block_symbols.rs`,
  `xtask/src/gen_tables.rs`, generated `crates/splot-core/src/tables/conversion.rs`,
  and table spot tests.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-ROADMAP.md`, generated feature/support/conformance status docs,
  and OpenSpec artifacts.
- No new dependencies, no crate dependency graph changes, no public API change,
  no validator diagnostics change, and no encoder work.
