## 1. Visible-AC tokens + row

- [x] 1.1 Add `general_intra_64x64_luma_visible_ac_tokens` (AC level 4 -> `coeff_base_eob` symbol 3, no `coeff_br`; DC `coeff_base` at the `Level[]`-derived context 2).
- [x] 1.2 Add the DC `coeff_base` `tx_size 4` context-2 row to `BlockSymbolTraceCdfRows` + `row_mut`.

## 2. Composer + oracle

- [x] 2.1 Add `compose_general_intra_visible_ac_block_trace` and `emit_minimal_intra_visible_ac_ivf()`.
- [x] 2.2 Cross-crate oracle: `splot decode` reconstructs a vertical cosine (each row constant; top 8 rows 129, middle 48 = 128, bottom 8 = 127) with flat 128 chroma.
- [x] 2.3 splot-encode tests: the 11-token trace order/symbols, roundtrip, distinct-from-level-1.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-GENERAL-INTRA-VISIBLE-AC` to the implementation matrix and refresh generated docs.
- [x] 3.2 Keep tracking honest: the first visibly non-flat reconstruction; level 4 is the largest no-`coeff_br` AC; larger AC magnitudes (needing the AC `coeff_br` context) and 2-D / higher-frequency coefficients remain follow-ups; decode-verified against splot-decode; not a general encoder or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused encode/cli tests, feature-status checks, and `cargo xtask ci`.
