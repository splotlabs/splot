## 1. General chroma coded tokens

- [x] 1.1 Add `general_intra_32x32_chroma_u_dc_coded_tokens` (`TX_32X32` `txb_skip`, `eob_pt_1024` at chroma `eob_ctx 2`, `coeff_base_lf_eob_uv`); move it + the general luma coded-DC tokens into a `general_coded` submodule.
- [x] 1.2 Route the chroma `eob_pt_1024` (`eob_ctx 2`) row through `BlockSymbolTraceCdfRows` + `row_mut`.

## 2. Composer + emit + oracle

- [x] 2.1 Add `compose_general_intra_coded_chroma_u_block_trace` (luma skip + U coded + U DC `sign_bit` bypass + V skip at `EobU != 0` context 6) and `emit_minimal_intra_coded_chroma_ivf()`.
- [x] 2.2 Cross-crate oracle: `splot decode` of the emitted IVF yields flat luma 128, flat U 127, flat V 128.
- [x] 2.3 splot-encode tests: the trace order/symbols, roundtrip, parseable single-frame IVF.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-GENERAL-INTRA-CODED-CHROMA-DC` to the implementation matrix and refresh generated docs.
- [x] 3.2 Keep tracking honest: the first decodable coded-chroma frame (U 127), decode-verified against `splot-decode`; not a general encoder or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused encode/cli tests, feature-status checks, and `cargo xtask ci`.
