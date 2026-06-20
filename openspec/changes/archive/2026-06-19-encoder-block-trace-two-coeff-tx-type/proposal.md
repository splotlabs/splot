## Why

The eob=2 trace (`ENC-INTRA-BLOCK-TRACE-TWO-COEFF`) assumed the no-transform-type
config (`reduced_tx_set == 2` / DCT-only) to avoid the `intra_tx_type` symbol. Now
that the `intra_tx_type` token exists (`ENC-INTRA-TX-TYPE-TOKEN`), this change
composes the general eob > 1 trace for the default `reduced_tx_set` `TX_SET_INTRA_1`
configuration — the first multi-coefficient trace that actually carries a
transform-type symbol.

## What Changes

- Add `ENC-INTRA-BLOCK-TRACE-TWO-COEFF-TX-TYPE` as a private `splot-encode`
  encoder-tool feature.
- Add `compose_minimal_intra_two_coeff_block_trace_with_tx_type()` in
  `block_symbol_trace`: the eob=2 trace with the §5.20.8.2 `intra_tx_type` DCT_DCT
  symbol (symbol 0) inserted after `eob_pt_16` (the position `transform_type()` is
  read, §5.20.7.27 line 15474). The eleven-token trace is
  `[0,0,0, 0, 1, 0, 0, 0, 0, 1, 1]`.
- Wire the 4x4 `TX_SET_INTRA_1` `intra_tx_type` row into `BlockSymbolTraceCdfRows`.
- Extract the §5.20.7.28 golomb-tail composers into a
  `block_symbol_trace/golomb.rs` submodule to keep the parent file under the
  1000-line source budget.
- Prove the trace roundtrips through one §8.2 coder and that the `intra_tx_type`
  symbol sits after `eob_pt_16`. It still assumes `enable_intra_ist == 0` (no
  `sec_tx_type`). No packet output.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the eob=2 multi-coefficient trace with the
  `TX_SET_INTRA_1` `intra_tx_type` symbol.

## Impact

- Affected code: `crates/splot-encode/src/block_symbol_trace.rs` (composer, CDF row,
  module split), `crates/splot-encode/src/block_symbol_trace/golomb.rs` (extracted
  golomb composers), `crates/splot-encode/src/block_symbol_trace_tests.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; crate-private, not re-exported.
- Dependency impact: none.
- Validator/CLI impact: none.
