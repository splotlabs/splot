## Why

The ac0ej3 mission stream now reaches the classified Wiener NS loop-restoration path, but the live runtime still stops before it has a bounded plan for the 10-bit `CurrFrame`/`CdefFrame` storage and `LrTxSkip` grid that §7.20.4 classification would consume. This change advances that frontier without inventing decoded samples or claiming loop-restoration filtering.

## What Changes

- Add Feature ID `DECODE-AC0EJ3-LR-RUNTIME-STORAGE-RETENTION` and matrix row `ac0ej3-lr-runtime-storage-retention`.
- Derive the live ac0ej3 loop-restoration storage footprint for two 10-bit frame buffers plus an `LrTxSkip` grid, and enforce the existing decode limits before any future allocation or output.
- Move the live ac0ej3 unsupported diagnostic to the next honest boundary: storage shapes are retained, but tile reconstruction has not populated the frame buffers or `LrTxSkip` values.
- Keep `DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-STORAGE` as the helper row for caller-supplied decoded-frame/grid-backed classification.

## Capabilities

### New Capabilities
- `ac0ej3-lr-runtime-storage-retention`: ac0ej3-specific fail-closed runtime planning for 10-bit loop-restoration frame buffers and `LrTxSkip` grid retention.

### Modified Capabilities
- `decoder-support`: records that the live ac0ej3 diagnostic advances from classified-Wiener storage helper wiring to runtime storage-retention planning.
- `ac0ej3-lr-classified-wiener-storage`: records that this row remains the storage-backed helper contract while the live ac0ej3 diagnostic is superseded by runtime storage-retention planning.

## Impact

- Affected code: `crates/splot-decode/src/runtime_minimal.rs`, `crates/splot-decode/src/runtime_minimal/wienerns_lr.rs`, focused runtime tests, CLI ac0ej3 ignored gate test, and decoder support/matrix docs.
- Diagnostics: `decode/unsupported-feature` changes from `unsupported_wienerns_lr_classified_wiener_runtime_storage` to a new runtime-storage-retention reason after limit checks pass; low storage limits remain `decode/resource-limit`.
- Non-goals: no decoded sample population, no `FilterClass` grid retention, no `SubclassLookup`, no §7.20.3/§7.20.4 filtering, no chroma Wiener NS filtering, no 10-bit output/hash/Y4M success, no reference refresh, and no AVM/dav2d equality claim.
