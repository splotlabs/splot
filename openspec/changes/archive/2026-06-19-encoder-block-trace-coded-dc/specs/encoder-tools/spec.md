## ADDED Requirements

### Requirement: Encoder minimal coded intra block trace

The encoder SHALL provide a private minimal coded intra block trace stage tracked
by `ENC-INTRA-BLOCK-TRACE-CODED-DC`, extending the `block_symbol_trace` module.
For the current minimal subset, the stage SHALL compose the ordered AV2 trace
`y_mode_set`, `y_mode_index`, `uv_mode` (§5.20.5.3), then the coded luma
`residual()` symbols for a single nonzero DC coefficient — `txb_skip == 0`,
`eob_pt_16`, `coeff_base_eob`, and `dc_sign` (§5.20.7.27) — then the all-zero U and
V `txb_skip` symbols, in `residual()` plane order. It SHALL prove the complete
nine-symbol sequence writes through one in-tree AV2 §8.2 symbol encoder and decodes
back through one symbol decoder with shared CDF state, using the neutral top-left
luma coefficient CDF rows (`eob_pt_16`, `coeff_base_lf_eob`, `dc_sign`) per §8.3.2.
The coded luma DC token accessor SHALL be proven equivalent to the coefficient
tokenizer's coded DC path. It SHALL NOT emit multi-coefficient blocks, coefficient
base-range / higher-frequency / sign-golomb extension syntax, chroma coefficients,
CfL/CCTX, partition syntax, tile payloads, coded packets, public CLI success, or
modes beyond the DC minimal tier.

#### Scenario: Coded trace is the mode prefix, coded luma residual, then chroma txb_skip

- **WHEN** the minimal coded intra DC block trace is composed
- **THEN** the trace SHALL be exactly the ordered `y_mode_set`, `y_mode_index`,
  `uv_mode` mode tokens, then the luma `txb_skip == 0`, `eob_pt_16`,
  `coeff_base_eob`, and `dc_sign` tokens, then the all-zero U and V `txb_skip`
  tokens.

#### Scenario: Coded trace roundtrips through one section 8.2 coder

- **WHEN** the composed coded trace is written through one in-tree AV2 section 8.2
  symbol encoder using the scoped mode and coefficient CDF rows
- **THEN** the bytes SHALL decode through one in-tree symbol decoder to the same
  ordered token symbols with shared CDF state
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Coded trace does not produce packets

- **WHEN** the coded intra block trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, multi-coefficient or chroma-coefficient syntax, or CLI success from the
  trace alone.
