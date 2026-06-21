## 1. 2-D tokens + row

- [x] 1.1 Add `general_intra_64x64_luma_2d_base_tokens` (two level-4 ACs, scan 1 + scan 2, zero DC).
- [x] 1.2 Add the DC `coeff_base` ctx-4 row to `BlockSymbolTraceCdfRows` + `row_mut`.

## 2. Composer + oracle

- [x] 2.1 Add `compose_general_intra_2d_block_trace` (two AC sign bypasses in reverse-scan order) + `emit_minimal_intra_2d_ivf()`.
- [x] 2.2 Cross-crate oracle: `splot decode` reconstructs a diagonal gradient (non-separable; 3x3 band grid [[128,127,127],[129,128,127],[129,129,128]]) with flat 128 chroma.
- [x] 2.3 splot-encode tests: the 14-token trace order/symbols, roundtrip, distinct-from-eob3.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-GENERAL-INTRA-2D` to the implementation matrix and refresh generated docs.
- [x] 3.2 Keep tracking honest: the first 2-D (non-separable) reconstruction + the first two-nonzero-non-EOB-sign block (asymmetric signs prove the reverse-scan bypass order); decode-verified against splot-decode; not a general encoder or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused encode/cli tests, feature-status checks, and `cargo xtask ci`.
