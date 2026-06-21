## 1. Implementation
- [x] 1.1 Add `tokenize_general_lf_luma_block` (scan + eob + reverse base pass +
      reverse interleaved sign pass + chroma close) in `general_walk.rs`.
- [x] 1.2 Add `recover_quant_from_tokens` (self-consistency inverse).
- [x] 1.3 Add `Error::CoefficientTokenizationUnsupportedEob` + the routed CoeffBaseLf
      4x4 ctx-2 CDF row from the generated splot-core table.

## 2. Tests
- [x] 2.1 Asymmetric eob=2 exact stream + derived (not pinned) DC context.
- [x] 2.2 Roundtrip + recover_quant == input.
- [x] 2.3 eob=1 matches the existing single-DC tokens.
- [x] 2.4 All-zero block → single all_zero.
- [x] 2.5 Deterministic recovery.
- [x] 2.6 Reject nonzero beyond scan index 1 / magnitude beyond 4.
- [x] 2.7 Negative sign-swap test guards the AC-before-DC contract.

## 3. Tracking
- [x] 3.1 Add the `ENC-COEFF-GENERAL-WALK-LF-BASE` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
