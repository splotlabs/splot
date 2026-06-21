## 1. Two-nonzero tokens

- [x] 1.1 Add `general_intra_64x64_luma_two_nonzero_tokens` (base pass + DC `dc_sign`; caller appends the AC `sign_bit`). No new CDF rows.

## 2. Composer + oracle

- [x] 2.1 Add `compose_general_intra_two_nonzero_block_trace` and `emit_minimal_intra_two_nonzero_ivf()`.
- [x] 2.2 Cross-crate oracle: `splot decode` reconstructs the cosine + DC offset (each row constant; top 14 rows 129, the rest 128) with flat 128 chroma.
- [x] 2.3 splot-encode tests: the 12-token trace order/symbols, roundtrip, distinct-from-visible-AC.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-GENERAL-INTRA-TWO-NONZERO` to the implementation matrix and refresh generated docs.
- [x] 3.2 Keep tracking honest: the first two-nonzero-coefficient block (DC `coeff_base` nonzero + the two-sign scan-order sign pass); decode-verified against splot-decode; not a general encoder or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused encode/cli tests, feature-status checks, and `cargo xtask ci`.
