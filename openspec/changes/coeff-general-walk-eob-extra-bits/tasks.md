## 1. Implementation
- [x] 1.1 Lift the walk to eob 5-10 (eobPt 4/5, `eob_extra` flag + MSB-first
      `eob_extra_bit` bypass literals); reject scan index ≥ 10; extend recovery.
- [x] 1.2 Verify the LF boundary (row+col < 4) so eob 1-10 reuse the hole-free banks
      with no new CDF rows.

## 2. Tests
- [x] 2.1 Exact-stream eob=6 (eobPt 4) and eob=10 (eobPt 5, `[0,1]` MSB-first); the
      every-refined-eob header table.
- [x] 2.2 Bounded exhaustive routing fuzz over eob 5-10.
- [x] 2.3 eob ≥ 11 (scan index ≥ 10) rejected.
- [x] 2.4 Split the test file by responsibility under the 1000-line budget.

## 3. Tracking
- [x] 3.1 Add the `ENC-COEFF-GENERAL-WALK-EOB-EXTRA-BITS` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
