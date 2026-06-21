## 1. Implementation
- [x] 1.1 Add splot-core `NUM_BASE_LEVELS = 2`; lift the walk to eob 11 with
      per-coefficient `is_lf` LF/HF selection and the HF level/magnitude caps.
- [x] 1.2 Add `CoeffBaseEob`/`CoeffBr` selectors + tokens + HF banks to both routers;
      `coeff_br_lf_luma_context` HF (no `+7`) branch.
- [x] 1.3 Split the §8.2 proof harness into `entropy_proof.rs`.

## 2. Tests
- [x] 2.1 Exact-stream eob=11 (HF EOB selector, LF coeffs unchanged); HF mag>base
      (`coeff_br_hf` ctx 0); HF cap rejection (mag 6 HF rejected, LF accepted).
- [x] 2.2 Bounded asymmetric-sign fuzz over eob=11; dual-router entropy-proof routing.
- [x] 2.3 eob ≥ 12 rejected.

## 3. Tracking
- [x] 3.1 Add the `ENC-COEFF-GENERAL-WALK-HF-EOB11` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
