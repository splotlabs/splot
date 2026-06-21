## 1. Implementation
- [x] 1.1 Lift the walk to eob 3-4 (`eob_pt_16` symbol `eobPt-1`, `eob_extra` flag);
      reject scan index ≥ 4; extend recovery.
- [x] 1.2 Add `eob_extra_token`; refactor the 4x4-LF CDF rows into context-indexed
      banks (hole-free, from the generated splot-core tables).

## 2. Tests
- [x] 2.1 eob=3 exact stream (eob_extra flag 0); eob=4 (flag 1).
- [x] 2.2 600-block exhaustive routing fuzz (positions × magnitude tiers × signs).
- [x] 2.3 eob ≥ 5 rejected.

## 3. Tracking
- [x] 3.1 Add the `ENC-COEFF-GENERAL-WALK-EOB-EXTRA` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
