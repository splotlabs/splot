## ADDED Requirements

### Requirement: Encoder coded chroma intra block trace

The encoder SHALL provide a private coded chroma intra block trace stage tracked
by `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC`, extending the coded DC block trace.
For a single nonzero chroma U DC coefficient, the U `residual()` SHALL emit
`txb_skip == 0`, `eob_pt_16`, `coeff_base_eob`, and `dc_sign` (§5.20.7.27) with the
§8.3.2 chroma contexts: eob context 2 (`eobCtx = (plane > 0) ? 2 : is_inter`), the
dedicated chroma `TileCoeffBaseLfEobUvCdf` at the DC context 0, and `dc_sign`
plane-type 1. The stage SHALL compose the AV2 §5.20.5.3 mode-info prefix, the coded
luma `residual()`, the coded U `residual()`, then the all-zero V `txb_skip`, in
`residual()` plane order, and SHALL prove the complete trace writes through one
in-tree AV2 §8.2 symbol encoder and decodes back through one symbol decoder with
shared CDF state. It SHALL NOT emit the chroma base-range/golomb tiers, V-plane
coded coefficients, multi-coefficient blocks, partition syntax, tile payloads,
coded packets, public CLI success, or modes beyond the DC minimal tier.

#### Scenario: Coded chroma trace is mode prefix, coded luma, coded U, then V txb_skip

- **WHEN** the minimal coded chroma intra DC block trace is composed
- **THEN** the trace SHALL be exactly the ordered `y_mode_set`, `y_mode_index`,
  `uv_mode` mode tokens, then the luma `txb_skip == 0`, `eob_pt_16`,
  `coeff_base_eob`, `dc_sign` tokens, then the U `txb_skip == 0`, `eob_pt_16`,
  `coeff_base_eob`, `dc_sign` tokens, then the all-zero V `txb_skip` token.

#### Scenario: Coded chroma trace roundtrips through one section 8.2 coder

- **WHEN** the composed coded chroma trace is written through one in-tree AV2
  section 8.2 symbol encoder using the scoped mode and per-plane coefficient CDF
  rows (including the chroma eob/base-eob/dc_sign rows)
- **THEN** the bytes SHALL decode through one in-tree symbol decoder to the same
  ordered token symbols with shared CDF state
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Coded chroma trace does not produce packets

- **WHEN** the coded chroma block trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, chroma base-range/golomb syntax, multi-coefficient syntax, or CLI
  success from the trace alone.
