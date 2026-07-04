## 1. CDF Plumbing

- [x] 1.1 Add `TileCctxTypeCdf` storage, selector routing, default copy, and lifecycle coverage.
- [x] 1.2 Add block-symbol read/update tests for the CCTX type selector.

## 2. Residual Handoff

- [x] 2.1 Add policy-scoped LR tx-skip handoff handling for chroma non-DCT transform-set syntax.
- [x] 2.2 Add policy-scoped CCTX type reads that record metadata for LR handoff while keeping reconstruction-safe callers fail-closed.
- [x] 2.3 Add focused positive and negative residual tests for the new handoff behavior.

## 3. Tracking And Validation

- [x] 3.1 Update implementation and decoder-support matrix rows plus regenerated tracking docs.
- [x] 3.2 Re-run the live local decoder mission probe and focused tests, then run OpenSpec and repo gates.
