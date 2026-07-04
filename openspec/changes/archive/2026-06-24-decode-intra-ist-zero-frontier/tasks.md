## 1. CDF Coverage

- [x] 1.1 Add `TileSecTxTypeCdf`, `TileMostProbableStxSetCdf`, and
  `TileMostProbableStxSetAdstCdf` rows to the tile CDF subset.
- [x] 1.2 Extend row-selection bounds, block-symbol read, update-mode, and
  lifecycle tests for the new CDF rows.

## 2. Intra IST Residual Syntax

- [x] 2.1 Replace the blanket intra IST residual rejection with spec-ordered
  `sec_tx_type` consumption for the covered DCT_DCT subset.
- [x] 2.2 Admit only `sec_tx_type == 0`; for non-zero intra IST, consume
  `most_probable_stx_set` where required and fail closed with a stable
  unsupported reason.
- [x] 2.3 Add focused residual tests for zero admission and active secondary
  transform rejection.

## 3. Tracking and Verification

- [x] 3.1 Add `DECODE-INTRA-IST-ZERO-FRONTIER` to the implementation
  and decoder-support matrices, update spec mapping if needed, and regenerate
  generated status docs.
- [x] 3.2 Run focused tests, the local decoder mission decode probe, OpenSpec
  validation, `cargo xtask feature-status`, `cargo xtask check-feature-status`,
  `cargo xtask check-decoder-support`, and `cargo xtask ci`.
- [x] 3.3 Sync and archive the OpenSpec change once all tasks are complete.
