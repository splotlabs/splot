## ADDED Requirements

### Requirement: Encoder coded chroma intra block trace

The encoder SHALL provide a private coded chroma intra block trace stage tracked
by `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC`, extending the coded DC block trace on
top of the §8.2.5 bypass-literal token. For a single nonzero chroma U DC
coefficient, the U `residual()` SHALL emit `txb_skip == 0`, `eob_pt_16`, and
`coeff_base_eob` as CDF symbols with the §8.3.2 chroma contexts (eob context 2, the
dedicated chroma `TileCoeffBaseLfEobUvCdf` at the DC context 0), and the U DC sign
SHALL be emitted as a `sign_bit` `L(1)` bypass literal (§5.20.7.27 codes the
`dc_sign` CDF only for the luma DC and `dc_sign_horz_vert` for the directional luma
axis signs). Because the coded U sets `EobU != 0`, the all-zero V `txb_skip` SHALL
use the §8.3.2 V context 6. The stage SHALL compose the §5.20.5.3 mode-info prefix,
the coded luma `residual()`, the coded U CDF `residual()`, the U `sign_bit` bypass
literal, then the all-zero V `txb_skip`, in `residual()` plane order, and SHALL
prove the complete trace writes through one in-tree AV2 §8.2 symbol encoder and
decodes back through one symbol decoder with shared CDF state. The coded chroma
coefficient-token accessor SHALL reject magnitudes outside the base tier with a
typed error. It SHALL NOT emit the chroma base-range/golomb tiers, V-plane coded
coefficients, multi-coefficient blocks, partition syntax, tile payloads, coded
packets, public CLI success, or modes beyond the DC minimal tier.

#### Scenario: Coded chroma trace orders CDF residual then sign_bit bypass then V txb_skip

- **WHEN** the minimal coded chroma intra DC block trace is composed
- **THEN** the trace SHALL be exactly the ordered mode tokens, the coded luma
  residual tokens, the coded U `txb_skip == 0` / `eob_pt_16` / `coeff_base_eob` CDF
  tokens, the U DC `sign_bit` bypass literal, then the all-zero V `txb_skip` token
  at the EobU context.

#### Scenario: Coded chroma trace roundtrips through one section 8.2 coder

- **WHEN** the composed coded chroma trace is written through one in-tree AV2
  section 8.2 symbol encoder using the scoped CDF rows and the bypass-literal path
- **THEN** the bytes SHALL decode through one in-tree symbol decoder to the same
  ordered values with shared CDF state
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Coded chroma magnitude is rejected outside the base tier

- **WHEN** the coded chroma U coefficient-token accessor is asked for a magnitude
  of 0 or one requiring `coeff_br` / the golomb tail
- **THEN** it SHALL return a typed unsupported-magnitude error rather than emit
  incomplete tokens.

#### Scenario: Coded chroma trace does not produce packets

- **WHEN** the coded chroma block trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, chroma base-range/golomb syntax, or CLI success from the trace alone.
