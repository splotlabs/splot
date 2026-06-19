## 1. eob=2 multi-coefficient trace

- [x] 1.1 Add the eob=2 AC `coeff_base_lf_eob` (ctx 1) and DC `coeff_base_lf` (ctx 1, TCQ-off) rows to `BlockSymbolTraceCdfRows` with routing.
- [x] 1.2 Add `compose_minimal_intra_two_coeff_block_trace()`: mode prefix + coded `all_zero` + `eob_pt_16=1` + AC `coeff_base_eob` (ctx 1) + DC `coeff_base` (derived ctx) + AC `sign_bit` bypass + all-zero U/V.
- [x] 1.3 Derive the DC `coeff_base` low-frequency context via `coeff_base_lf_luma_context` from the AC's `Level[]` (not a hard-coded literal); assert it equals the routed context.

## 2. Tests

- [x] 2.1 Prove the trace is the mode prefix, coded `all_zero`, `eob_pt_16`, AC `coeff_base_eob` (ctx 1), DC `coeff_base` (derived ctx 1), AC `sign_bit` bypass, then all-zero U/V — symbols `[0,0,0,0,1,0,0,0,1,1]`.
- [x] 2.2 Prove the trace roundtrips deterministically through one §8.2 coder.
- [x] 2.3 Prove the DC `coeff_base` token carries the derived low-frequency context 1.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-TRACE-TWO-COEFF` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming eob > 2, chroma multi-coefficient, partition syntax, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
