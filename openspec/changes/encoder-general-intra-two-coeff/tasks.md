## 1. eob=2 multi-coefficient tokens

- [x] 1.1 Add `general_intra_64x64_luma_two_coeff_tokens` (`txb_skip=0`, `eob_pt_1024=1`, AC `coeff_base_eob` ctx 1, DC `coeff_base` ctx 1) at `TX_64X64`; the 32x32 scan / `Level[]` context match the minimal tier.
- [x] 1.2 Add the AC `coeff_base_eob` and DC `coeff_base` `tx_size 4` rows to `BlockSymbolTraceCdfRows` + `row_mut`.

## 2. Composer + oracle

- [x] 2.1 Add `compose_general_intra_two_coeff_block_trace` (do_split + modes + eob=2 luma + AC sign bypass + U/V skip) and `emit_minimal_intra_two_coeff_ivf()`.
- [x] 2.2 Cross-crate oracle: `splot decode` validates the eob=2 stream (exit_symbol) and reconstructs 6144 bytes; a splot-encode test asserts the IVF differs from the skip frame.
- [x] 2.3 splot-encode tests: the 11-token trace order/symbols, roundtrip.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-GENERAL-INTRA-TWO-COEFF` to the implementation matrix and refresh generated docs.
- [x] 3.2 Keep tracking honest: the first eob>1 frame; the level-1 AC reconstructs sub-visibly (flat 128); a visibly-non-flat AC (per-level DC context) is a follow-up; decode-verified against splot-decode; not a general encoder or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused encode/cli tests, feature-status checks, and `cargo xtask ci`.
