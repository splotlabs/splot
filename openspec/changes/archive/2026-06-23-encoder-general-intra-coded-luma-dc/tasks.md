## 1. The eob_pt_1024 token + general coded-DC tokens

- [x] 1.1 Add `CoefficientTokenSyntax::EobPt1024` and `CoefficientCdfRowSelector::EobPt1024` (+ the `closed_loop` no-op match arm).
- [x] 1.2 Add `general_intra_64x64_luma_dc_coded_tokens(q, magnitude, negative)` at the `TX_64X64` contexts (`eob_pt_1024`, `TX_64X64` `coeff_base_lf_eob`; reusing the shared `coeff_br` / `dc_sign` rows).
- [x] 1.3 Route the `eob_pt_1024` and `TX_64X64` `coeff_base_lf_eob` rows through `BlockSymbolTraceCdfRows` + `row_mut`.

## 2. Composer + emit + oracle

- [x] 2.1 Add `compose_general_intra_dc_coded_block_trace(magnitude, negative)` (do_split + modes + coded luma + U/V skip) and `emit_minimal_intra_coded_dc_ivf()` at magnitude 6 (sub-golomb).
- [x] 2.2 Cross-crate oracle: `splot decode` of the emitted IVF yields flat luma `127` and flat chroma `128`.
- [x] 2.3 splot-encode tests: the trace order/symbols, roundtrip, the `eob_pt_1024` selector, and a parseable single-frame IVF.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-GENERAL-INTRA-CODED-LUMA-DC` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Keep tracking honest: the first decodable coded-DC frame (luma 127), decode-verified against `splot-decode`; magnitude 7 (the q80 luma value, 100) needs the § 5.20.7.28 golomb tail not yet modeled; not a general encoder or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused encode/cli tests, feature-status checks, and `cargo xtask ci`.
