## 1. Implementation
- [x] 1.1 `TxGeom`/`EobPtKind` + one size-generic walk codepath (4×4 delegates, byte-identical).
- [x] 1.2 `tokenize_general_16x16_luma_block` eob 1..=32; reject eob>32.

## 2. Tests
- [x] 2.1 §8.2 roundtrip over asymmetric 16×16 blocks; band-break assertion; 4×4 suite green.

## 3. Tracking
- [x] 3.1 Add the `ENC-COEFF-TOKENIZE-16X16-BASE` matrix row.
- [x] 3.2 Regenerate feature status; run `cargo xtask ci`.
