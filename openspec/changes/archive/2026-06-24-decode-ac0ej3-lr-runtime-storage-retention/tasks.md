## 1. Runtime Frontier

- [x] 1.1 Add `DECODE-AC0EJ3-LR-RUNTIME-STORAGE-RETENTION` constants and a typed storage-retention summary for two 10-bit frame buffers plus the `LrTxSkip` grid.
- [x] 1.2 Derive and limit-check the live storage footprint from parsed sequence/frame facts without populating fake decoded samples or fake `LrTxSkip` values.
- [x] 1.3 Move the live ac0ej3 unsupported diagnostic to the new runtime-storage-retention row after retention limits pass.

## 2. Tests And Docs

- [x] 2.1 Add focused tests for storage-retention summary derivation, storage limit failure, and the new unsupported diagnostic.
- [x] 2.2 Update the ignored local ac0ej3 CLI test and support/matrix docs to reference the new diagnostic and Feature ID.
- [x] 2.3 Run focused tests plus `openspec validate`, `cargo xtask feature-status`, `cargo xtask check-feature-status`, and `cargo xtask ci`.
