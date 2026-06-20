## 1. Coded V tokens + composer

- [x] 1.1 Add `general_intra_32x32_chroma_v_dc_coded_tokens` (`VTxbSkip` coded at neutral ctx 0, `eob_pt_1024` chroma `eob_ctx 2`, `coeff_base_lf_eob_uv`); reuse existing rows.
- [x] 1.2 Add `compose_general_intra_coded_chroma_v_block_trace` (luma skip + U skip + V coded + V DC `sign_bit` bypass) and `emit_minimal_intra_coded_chroma_v_ivf()`.

## 2. Oracle + tests

- [x] 2.1 Cross-crate oracle: `splot decode` of the emitted IVF yields flat luma 128, flat U 128, flat V 127.
- [x] 2.2 splot-encode tests: trace order/symbols, roundtrip, parseable single-frame IVF.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-GENERAL-INTRA-CODED-CHROMA-V-DC` to the implementation matrix and refresh generated docs.
- [x] 3.2 Keep tracking honest: completes the per-plane coded-DC set; decode-verified against `splot-decode`; not a general encoder or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused encode/cli tests, feature-status checks, and `cargo xtask ci`.
