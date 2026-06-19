## 1. eob=2 trace with intra_tx_type

- [x] 1.1 Add the 4x4 `TX_SET_INTRA_1` `intra_tx_type` row to `BlockSymbolTraceCdfRows` with routing.
- [x] 1.2 Add `compose_minimal_intra_two_coeff_block_trace_with_tx_type()` — the eob=2 trace with the `intra_tx_type` DCT_DCT symbol (0) inserted after `eob_pt_16` (index 4).
- [x] 1.3 Extract the golomb-tail composers into a `block_symbol_trace/golomb.rs` submodule to keep the parent under the 1000-line budget.

## 2. Tests

- [x] 2.1 Prove the trace is the eob=2 trace plus one `intra_tx_type` (DCT_DCT, ctx tx_size_sqr 0) token after `eob_pt_16` — symbols `[0,0,0,0,1,0,0,0,0,1,1]`.
- [x] 2.2 Prove the trace roundtrips deterministically through one §8.2 coder.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-TRACE-TWO-COEFF-TX-TYPE` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming `sec_tx_type`, eob > 2, non-`TX_SET_INTRA_1` transform types, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
