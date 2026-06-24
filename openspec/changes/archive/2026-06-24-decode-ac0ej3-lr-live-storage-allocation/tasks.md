## 1. Runtime Storage Shells

- [x] 1.1 Add private live Wiener NS LR storage-shell types for unpopulated `CurrFrame`, `CdefFrame`, and `LrTxSkip` state.
- [x] 1.2 Allocate the shells from the existing storage-retention frontier after limit checks, without fabricating decoded samples or `LrTxSkip` values.
- [x] 1.3 Add focused tests for ten-bit 4:2:0 shell dimensions, unpopulated counts, and limit failure ordering.

## 2. Live Diagnostic

- [x] 2.1 Add `DECODE-AC0EJ3-LR-LIVE-STORAGE-ALLOCATION` constants and diagnostic reason.
- [x] 2.2 Move the live ac0ej3 failure from runtime storage-retention to live storage-allocation after shell allocation succeeds.
- [x] 2.3 Update the ignored local ac0ej3 CLI gate test to expect the new Feature ID, matrix row, reason, and diagnostic message.

## 3. Tracking And Verification

- [x] 3.1 Add implementation-matrix and decoder-support rows for the new feature, and forward-point the runtime-storage-retention row.
- [x] 3.2 Regenerate generated feature/status/coverage docs.
- [x] 3.3 Run focused runtime/CLI tests, `openspec validate --all --no-interactive`, `cargo xtask check-feature-status`, `cargo xtask conformance`, and `cargo xtask ci`.
