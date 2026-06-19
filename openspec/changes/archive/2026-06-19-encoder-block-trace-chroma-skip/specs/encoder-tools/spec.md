## ADDED Requirements

### Requirement: Encoder complete all-zero intra block trace

The encoder SHALL provide a private complete all-zero intra block trace stage
tracked by `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP`, extending the `block_symbol_trace`
module. For the current minimal subset, the stage SHALL compose the ordered AV2
trace `y_mode_set`, `y_mode_index`, `uv_mode` (§5.20.5.3), then the per-plane
`txb_skip` (`all_zero == 1`) symbols for luma, U, and V (§5.20.7.27, in
`residual()` plane order), and SHALL prove the complete six-symbol sequence writes
through one in-tree AV2 §8.2 symbol encoder and decodes back through one symbol
decoder with shared CDF state. Per §8.3.2
`TileTxbSkipCdf[is_inter || fsc_mode][txSzCtx][ctx]`, the U `txb_skip` SHALL use
the same bank as luma (the first index is `is_inter || fsc_mode` = 0 for this
intra non-FSC block, not plane type) at the §8.3.2 neutral context 6, and the V
`txb_skip` SHALL use the dedicated `TileVTxbSkipCdf` at context 0. It SHALL NOT emit non-all-zero
coefficient symbols, CfL/CCTX, partition syntax, tile payloads, coded packets,
public CLI success, or modes beyond the DC minimal tier.

#### Scenario: Complete trace is the mode prefix then per-plane txb_skip

- **WHEN** the minimal complete all-zero intra DC block trace is composed
- **THEN** the trace SHALL be exactly the ordered `y_mode_set`, `y_mode_index`,
  `uv_mode` mode tokens followed by the luma, U, and V `txb_skip` all-zero tokens.

#### Scenario: Complete trace roundtrips through one section 8.2 coder

- **WHEN** the composed complete trace is written through one in-tree AV2 section
  8.2 symbol encoder using the scoped mode and per-plane `txb_skip` CDF rows
- **THEN** the bytes SHALL decode through one in-tree symbol decoder to the same
  ordered token symbols with shared CDF state
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Complete trace does not produce packets

- **WHEN** the complete all-zero block trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, non-all-zero coefficient syntax, or CLI success from the trace alone.
