## 1. EobExtra token + rows

- [x] 1.1 Add `CoefficientTokenSyntax::EobExtra` + `CoefficientCdfRowSelector::EobExtra` + the `closed_loop` no-op arm.
- [x] 1.2 Add the `eob_extra` and scan-index-1 `coeff_base` ctx-9 rows to `BlockSymbolTraceCdfRows` + `row_mut`.

## 2. Tokens + composer + oracle

- [x] 2.1 Add `general_intra_64x64_luma_eob3_base_tokens` (eob=3 base pass: eob_pt_1024=2, eob_extra=0, reverse-scan base pass).
- [x] 2.2 Add `compose_general_intra_eob3_block_trace` + `emit_minimal_intra_eob3_ivf()` in `multi_coeff.rs`.
- [x] 2.3 Cross-crate oracle: `splot decode` reconstructs a horizontal cosine (each column constant; left 8 cols 129, middle 48 = 128, right 8 = 127) with flat 128 chroma.
- [x] 2.4 splot-encode tests: the 13-token trace order/symbols, roundtrip, distinct-from-visible-AC.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-GENERAL-INTRA-EOB3` to the implementation matrix and refresh generated docs.
- [x] 3.2 Keep tracking honest: the first eob>2 frame + the `eob_extra` CDF symbol; decode-verified against splot-decode; not a general encoder or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused encode/cli tests, feature-status checks, and `cargo xtask ci`.
