## 1. Implementation
- [x] 1.1 Generalize the golomb helper to `GolombParams` (general `m`); thread
      `hrLevelAvg` across emit/validate/recover.
- [x] 1.2 Remove the multiple-golomb rejection + the unused error variant; per-`m` cap.

## 2. Tests
- [x] 2.1 Exact-stream two-golomb (m driven above 1, hand-verified); high-`m` prefix.
- [x] 2.2 Bounded asymmetric-sign fuzz over 2-3 golomb coefficients.
- [x] 2.3 Single-golomb path byte-identical.

## 3. Tracking
- [x] 3.1 Add the `ENC-COEFF-GENERAL-WALK-GOLOMB-MULTI` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
