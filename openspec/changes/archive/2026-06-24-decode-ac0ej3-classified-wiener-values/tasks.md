## 1. Tracking

- [x] 1.1 Add `DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-VALUES` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the matching decoder support matrix row and diagnostic boundary text.

## 2. Runtime Frontier

- [x] 2.1 Add a value-backed classified-Wiener helper that adapts retained LR block facts into `splot-recon::pc_wiener_classify`.
- [x] 2.2 Update the live ac0ej3 runtime diagnostic to report the new storage boundary without claiming real frame or `LrTxSkip` reads.

## 3. Verification

- [x] 3.1 Add focused tests for supplied-value `FilterClass` derivation and invalid `LrTxSkip` propagation.
- [x] 3.2 Verify the local ac0ej3 fixture reaches the new diagnostic.
- [x] 3.3 Run focused tests plus `cargo xtask check-feature-status` and `cargo xtask check-decoder-support`.
- [x] 3.4 Run `cargo xtask feature-status` and `cargo xtask ci` before completion.
