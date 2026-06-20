## 1. All-planes composer

- [x] 1.1 Parameterize `general_intra_32x32_chroma_v_dc_coded_tokens` by the V `txb_skip` context (V-only caller passes 0; all-planes passes 6).
- [x] 1.2 Add `compose_general_intra_all_planes_coded_block_trace` (coded luma + coded U + U sign + coded V at ctx 6 + V sign) and `emit_minimal_intra_all_planes_coded_ivf()`.

## 2. Oracle + tests

- [x] 2.1 Cross-crate oracle: `splot decode` of the emitted IVF yields every plane flat at 127.
- [x] 2.2 splot-encode tests: the 17-token trace order/symbols, roundtrip, parseable single-frame IVF.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-GENERAL-INTRA-ALL-PLANES-CODED` to the implementation matrix and refresh generated docs.
- [x] 3.2 Keep tracking honest: all three planes coded at once with sub-golomb magnitudes; decode-verified against `splot-decode`; not byte-exact q80 (needs golomb), not a general encoder or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused encode/cli tests, feature-status checks, and `cargo xtask ci`.
