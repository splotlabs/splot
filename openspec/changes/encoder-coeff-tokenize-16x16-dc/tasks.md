## 1. Implementation
- [x] 1.1 `TX_SIZE_16X16_CTX` + `EobPt256` selector/syntax + `general_intra_16x16_luma_dc_coded_tokens`.
- [x] 1.2 New TX_16X16 banks + `EobPt256` arm in both CDF routers.

## 2. Tests
- [x] 2.1 §8.2 roundtrip of the 16×16 DC (asymmetric value) — luma stream + full block.

## 3. Tracking
- [x] 3.1 Add the `ENC-COEFF-TOKENIZE-16X16-DC` matrix row.
- [x] 3.2 Regenerate feature status; run `cargo xtask ci`.
