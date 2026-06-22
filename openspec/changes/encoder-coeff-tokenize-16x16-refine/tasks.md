## 1. Implementation
- [x] 1.1 `eob_pt_256` symbol 7 + `eob_pt_extra` bit (eobPt-8) for eobPt 8/9.
- [x] 1.2 Separate full-range entry; base entry contract unchanged.

## 2. Tests
- [x] 2.1 §8.2 roundtrip + hand-asserted eobPt-8/9 sequences; base-pass byte-identity.

## 3. Tracking
- [x] 3.1 Add the `ENC-COEFF-TOKENIZE-16X16-REFINE` matrix row.
- [x] 3.2 Regenerate feature status; run `cargo xtask ci`.
