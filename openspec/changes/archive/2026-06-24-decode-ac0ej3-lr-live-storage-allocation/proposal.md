## Why

The live ac0ej3 path now derives the 10-bit `CurrFrame`/`CdefFrame` storage
footprint and frame-wide `LrTxSkip` grid shape, but it still has no runtime
state that can explicitly represent “allocated but not populated” loop-
restoration storage. This change advances that frontier without fabricating
decoded samples or claiming loop-restoration output.

## What Changes

- Add Feature ID `DECODE-AC0EJ3-LR-LIVE-STORAGE-ALLOCATION` and matrix row
  `ac0ej3-lr-live-storage-allocation`.
- Introduce private live Wiener NS LR storage shells for the active-bit-depth
  `CurrFrame` and `CdefFrame` buffers plus the frame-wide `LrTxSkip` grid.
- Track the allocated storage shapes and unpopulated state explicitly, and keep
  storage-backed classification unreachable until real tile reconstruction and
  transform records populate the shells.
- Move the live ac0ej3 unsupported diagnostic to the next honest boundary:
  storage shells exist, but decoded samples and `LrTxSkip` values are still
  absent.

## Capabilities

### New Capabilities
- `ac0ej3-lr-live-storage-allocation`: ac0ej3-specific fail-closed runtime
  allocation for unpopulated 10-bit loop-restoration frame buffers and
  `LrTxSkip` grid storage.

### Modified Capabilities
- `decoder-support`: records that the live ac0ej3 diagnostic advances from
  runtime storage-retention planning to explicit live storage allocation.
- `ac0ej3-lr-runtime-storage-retention`: records that this row remains the
  storage-footprint planning contract while the live ac0ej3 diagnostic is
  superseded by live storage allocation.

## Impact

- Affected code: `crates/splot-decode/src/runtime_minimal.rs`,
  `crates/splot-decode/src/runtime_minimal/wienerns_lr.rs`, focused runtime
  tests, the ignored local ac0ej3 CLI gate test, and decoder
  support/feature-status docs.
- Diagnostics: the live ac0ej3 `decode/unsupported-feature` reason advances
  from `unsupported_wienerns_lr_runtime_storage_unpopulated` to a new live
  storage-allocation reason after limit checks and shell allocation succeed.
- Non-goals: no decoded sample population, no real `LrTxSkip` values, no
  `FilterClass` grid retention, no `SubclassLookup`, no §7.20.3/§7.20.4
  filtering, no chroma Wiener NS filtering, no 10-bit output/hash/Y4M success,
  no reference refresh, and no AVM/dav2d equality claim.
