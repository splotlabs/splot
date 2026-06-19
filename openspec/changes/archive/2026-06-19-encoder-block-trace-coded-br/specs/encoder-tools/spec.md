## ADDED Requirements

### Requirement: Encoder coded base-range intra block trace

The encoder SHALL provide a private coded base-range intra block trace stage
tracked by `ENC-INTRA-BLOCK-TRACE-CODED-BR`, extending the coded DC block trace.
For a single nonzero luma DC coefficient whose AV2 § 5.20.7.27 base level
(`coeff_base_eob + 1`) exceeds `LF_NUM_BASE_LEVELS`, the coded luma `residual()`
SHALL additionally emit a `coeff_br` symbol equal to
`magnitude - (LF_NUM_BASE_LEVELS + 1)` after `coeff_base_eob` (which saturates at
level 5), supporting magnitudes up to `LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE`.
Magnitude `LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1` (the § 5.20.7.27 `maxLevel`)
reaches the § 5.20.7.28 `read_quant` golomb threshold and SHALL be rejected until
the golomb tail is modeled, rather than emitting an incomplete token stream.
The `coeff_br` SHALL use the low-frequency `TileCoeffBrLfCdf` at the DC context 0
(§ 8.3.2). The coded DC token shape SHALL have a single source proven equivalent to
the coefficient tokenizer across the full supported magnitude range. The stage
SHALL prove the complete base-range coded-block trace writes through one in-tree
AV2 § 8.2 symbol encoder and decodes back through one symbol decoder with shared
CDF state. It SHALL NOT emit the golomb tail for magnitudes beyond the base-range
tier, multi-coefficient blocks, chroma coefficients, partition syntax, tile
payloads, coded packets, public CLI success, or modes beyond the DC minimal tier.

#### Scenario: Base-range coefficient emits coeff_br after coeff_base_eob

- **WHEN** a coded luma DC coefficient has magnitude in the base-range tier
  (greater than `LF_NUM_BASE_LEVELS`)
- **THEN** the coded luma `residual()` tokens SHALL be `txb_skip == 0`,
  `eob_pt_16`, `coeff_base_eob` saturated at its maximum level, then a `coeff_br`
  symbol equal to `magnitude - (LF_NUM_BASE_LEVELS + 1)`, then `dc_sign`.

#### Scenario: Base-range coded trace roundtrips through one section 8.2 coder

- **WHEN** the composed base-range coded block trace is written through one
  in-tree AV2 section 8.2 symbol encoder using the scoped mode and coefficient CDF
  rows (including the low-frequency `coeff_br` row)
- **THEN** the bytes SHALL decode through one in-tree symbol decoder to the same
  ordered token symbols with shared CDF state
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Coded token shape stays single-sourced

- **WHEN** the coded DC token shape is produced for any supported magnitude
- **THEN** the trace accessor and the coefficient tokenizer SHALL produce the same
  ordered tokens, asserted by an equivalence test over the full magnitude range.

#### Scenario: Base-range trace does not produce packets

- **WHEN** the base-range coded block trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, the golomb tail, multi-coefficient or chroma-coefficient syntax, or CLI
  success from the trace alone.
