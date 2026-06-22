## 1. Implementation
- [x] 1.1 Add `coeff_base_hf_luma_context`; lift the walk to eob 16 with per-coefficient
      LF/HF `coeff_base` selection for non-EOB coefficients.
- [x] 1.2 Add the `CoeffBase` selector + `coeff_base_hf_token` + the HF `coeff_base` bank
      (20 contexts) to both proof routers.

## 2. Tests
- [x] 2.1 Exact-stream eob=12 (non-EOB HF selector, LF unchanged); HF mag>base
      (`coeff_br_hf`); HF cap rejection at a non-EOB HF position.
- [x] 2.2 Bounded asymmetric-sign fuzz over eob 12-16; dual-router routing; hole-free HF
      `coeff_base` context sweep.
- [x] 2.3 The full eob-16 scan roundtrips.

## 3. Tracking
- [x] 3.1 Add the `ENC-COEFF-GENERAL-WALK-HF-MULTI` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
