## 1. Implementation
- [x] 1.1 Split the §8.2 recovery half into `general_walk_recover.rs`; add
      `general_walk_golomb.rs` (emission + recovery).
- [x] 1.2 Emit the m=1 golomb tail in `compose_sign_pass`; lift
      `validate_general_lf_scope` (single golomb coefficient, `x ≤ 517`).

## 2. Tests
- [x] 2.1 Exact bypass-stream tests (LF finite-q, LF prefix incl. 525, HF finite-q).
- [x] 2.2 Bounded asymmetric-sign fuzz over a single golomb coefficient across positions
      and magnitudes.
- [x] 2.3 Reject over-length and two-golomb-coefficient blocks.

## 3. Tracking
- [x] 3.1 Add the `ENC-COEFF-GENERAL-WALK-GOLOMB` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
