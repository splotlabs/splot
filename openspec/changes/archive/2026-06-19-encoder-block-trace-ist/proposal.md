## Why

Transform-type signaling Sub-brick C2: after the `sec_tx_type` token
(`ENC-SEC-TX-TYPE-TOKEN`), this change composes the eob=2 trace that actually carries
both §5.20.8.2 transform-type symbols — `intra_tx_type` AND the `sec_tx_type` IST
symbol — completing the transform-type prefix for a general intra block.

## What Changes

- Add `ENC-INTRA-BLOCK-TRACE-IST` as a private `splot-encode` encoder-tool feature.
- Add `compose_minimal_intra_two_coeff_block_trace_with_ist()` in
  `block_symbol_trace`: the tx-type trace (#349) with the `sec_tx_type` IST symbol
  (symbol 0, IST off) inserted right after `intra_tx_type` — the position
  `sec_tx_type` is read (§5.20.8.2 line 16613). The twelve-token trace is
  `[0,0,0, 0, 1, 0, 0, 0, 0, 0, 1, 1]`.
- Wire the 4x4 intra `TileSecTxTypeCdf[0]` row into `BlockSymbolTraceCdfRows`.
- Prove the trace roundtrips through one §8.2 coder and that `sec_tx_type` sits right
  after `intra_tx_type`. This block satisfies the IST condition (`enable_intra_ist ==
  1`, `eob 2 != 1`, `!Lossless`, `TxType == DCT_DCT`, `YMode != PAETH`, `eob 2 <=
  eobLim = IST_4X4_HEIGHT = 8`). It uses `sec_tx_type = 0` (no `most_probable_stx_set`).
  No packet output.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the eob=2 trace with both transform-type
  symbols (`intra_tx_type` + `sec_tx_type`).

## Impact

- Affected code: `crates/splot-encode/src/block_symbol_trace.rs` (composer, CDF row,
  module doc), `crates/splot-encode/src/block_symbol_trace_tests.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; crate-private, not re-exported.
- Dependency impact: none.
- Validator/CLI impact: none.
