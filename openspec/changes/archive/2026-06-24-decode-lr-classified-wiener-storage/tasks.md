## 1. Tracking

- [x] 1.1 Add `DECODE-LR-CLASSIFIED-WIENER-STORAGE` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the matching decoder support matrix row and live diagnostic boundary text.

## 2. Storage Handoff

- [x] 2.1 Make `splot-recon::pc_wiener_classify` propagate fallible `LrTxSkip` lookups.
- [x] 2.2 Add decoder helpers for frame-backed classified-Wiener source reads and bounded `LrTxSkip` grid lookups.
- [x] 2.3 Update the live local decoder mission runtime diagnostic to reference the storage frontier without claiming filtering/output.

## 3. Verification

- [x] 3.1 Add focused tests for storage-backed `FilterClass` derivation and typed `LrTxSkip` storage failures.
- [x] 3.2 Verify the local decoder mission fixture reaches the new diagnostic.
- [x] 3.3 Run focused tests plus `cargo xtask check-feature-status` and `cargo xtask check-decoder-support`.
- [x] 3.4 Run `cargo xtask feature-status` and `cargo xtask ci` before completion.
