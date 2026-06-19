## 1. eob=2 trace with intra_tx_type + sec_tx_type

- [x] 1.1 Add the 4x4 intra `TileSecTxTypeCdf[0]` row to `BlockSymbolTraceCdfRows` with routing.
- [x] 1.2 Add `compose_minimal_intra_two_coeff_block_trace_with_ist()` — the tx-type trace with the `sec_tx_type` IST-off symbol (0) inserted right after `intra_tx_type`.

## 2. Tests

- [x] 2.1 Prove the trace is the tx-type trace plus one `sec_tx_type` (IST, ctx tx_size_sqr 0) token right after `intra_tx_type` — symbols `[0,0,0,0,1,0,0,0,0,0,1,1]`.
- [x] 2.2 Prove the trace roundtrips deterministically through one §8.2 coder.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-TRACE-IST` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming `most_probable_stx_set`, eob > 2, the runtime IST-condition evaluation, non-`TX_SET_INTRA_1` transform types, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
