## Why

The ac0ej3 LR runtime now sizes the `LrTxSkip[y >> 2][x >> 2]` storage but still has
no value-backed way to retain the boolean grid required by AV2 §7.20.4
classified-Wiener filtering. Before the runtime can safely call the storage-backed
classifier, the decoder needs a helper that accepts real transform-block skip/eob
facts, proves the grid is complete, and rejects holes instead of inventing default
values.

## What Changes

- Add Feature ID `DECODE-AC0EJ3-LR-TX-SKIP-GRID-RETENTION` for a decoder-local
  `LrTxSkip` grid-retention primitive.
- Add a checked transform-record input that derives `LrTxSkip = skip_flag || eob == 0`
  per AV2 §5.20.7.25/§5.20.7.27/§7.20.4 and fills every covered 4x4 luma cell.
- Require complete grid coverage before constructing the existing bounded
  `WienerNsLrTxSkipGrid`; missing cells and out-of-range records stay structured
  reconstruction errors.
- Update decoder support/status and the implementation matrix for the new partial
  ac0ej3 LR prerequisite row.
- Keep the live ac0ej3 runtime fail-closed before decoded samples, `FilterClass`
  grid retention, LR filtering, output, reference refresh, or byte equality.

## Capabilities

### New Capabilities

- `ac0ej3-lr-tx-skip-grid-retention`: value-backed `LrTxSkip` grid-retention
  helper for the ac0ej3 Wiener NS LR path.

### Modified Capabilities

- `decoder-support`: add a partial support row for
  `DECODE-AC0EJ3-LR-TX-SKIP-GRID-RETENTION`.

## Impact

- Affects `crates/splot-decode/src/runtime_minimal/wienerns_lr.rs` and focused
  runtime-minimal LR tests.
- Updates OpenSpec artifacts plus `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, and generated decoder support status.
- No new dependencies, public API, crate graph, encoder behavior, AV2 syntax
  invention, or successful ac0ej3 decode claim.
