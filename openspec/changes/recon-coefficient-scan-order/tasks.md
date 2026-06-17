## 1. Spec + matrix

- [x] 1.1 Add the `RECON-COEFFICIENT-SCAN-ORDER` row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the `coefficient-scan-order` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the row + feature id to the `reconstruction-transform-and-filters`
  coverage group in `xtask/src/decoder_conformance_coverage.rs`.
- [x] 1.4 Add the OpenSpec `decoder-support` ADDED delta for this change.

## 2. Implementation

- [x] 2.1 Add `coefficient_scan.rs` with the `TransformClass` enum and
  `coefficient_scan_order(w, h, class, out)` (raster for VERT/HORIZ, anti-diagonal
  for 2D, signed-i32 2D logic).
- [x] 2.2 Add `ReconError::InvalidScanShape` / `ReconError::ScanLengthMismatch`
  variants + `Display` arms.
- [x] 2.3 Register the module + export `TransformClass` / `coefficient_scan_order`
  in `lib.rs`; update the crate `//!` lists + feature-id list.

## 3. Tests

- [x] 3.1 Hand-traced 4x4 2D scan, VERT identity, HORIZ transpose, permutation
  validity across all 16 shapes x 3 classes, and shape/length rejection.

## 4. Docs + gate

- [x] 4.1 Update `docs/DECODER-ROADMAP.md`.
- [x] 4.2 Regenerate the four generated status docs.
- [x] 4.3 `cargo xtask ci` green.
