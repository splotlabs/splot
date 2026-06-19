## ADDED Requirements

### Requirement: Encoder non-EOB coeff_base low-frequency luma token

The encoder SHALL provide a private non-EOB `coeff_base` low-frequency luma token
tracked by `ENC-COEFF-BASE-LF-TOKEN`. It SHALL add the `CoeffBase` token syntax and
a `CoeffBaseLf` CDF-row selector for `TileCoeffBaseLfCdf[coeff_cdf_q_ctx][tx_size]
[ctx][tcq_ctx]`, and a `coeff_base_lf_token(coeff_cdf_q_ctx, ctx, tcq_ctx, level)`
accessor whose symbol equals the non-EOB base `level`. The token SHALL roundtrip
through one in-tree AV2 §8.2 symbol encoder/decoder via the generic
coefficient-token CDF-row router at the eob=2 trace's DC context (the
`coeff_base_lf_luma_context` result for an AC level-1 neighbour) and the TCQ-off
context. It SHALL NOT compose a multi-coefficient trace, derive chroma /
high-frequency `coeff_base`, emit `coeff_br` for the AC, or produce a coded packet.

#### Scenario: coeff_base_lf token carries the non-EOB base level

- **WHEN** a `coeff_base_lf` token is built for base level `L`
- **THEN** its syntax SHALL be `coeff_base`, its symbol SHALL equal `L` (not
  `L + 1`), and it SHALL select the `TileCoeffBaseLfCdf` row at the eob=2 DC context
  and the TCQ-off context.

#### Scenario: coeff_base_lf token roundtrips through the §8.2 coder

- **WHEN** `coeff_base_lf` tokens for several base levels are roundtripped through
  the generic coefficient-token router and one in-tree AV2 §8.2 coder
- **THEN** each decoded symbol SHALL equal the encoded base level.

#### Scenario: The token is not yet composed into a trace

- **WHEN** the `coeff_base_lf` token is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a multi-coefficient trace or
  Baseline Encoder Profile v1 output from the token alone.
