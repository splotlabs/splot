## ADDED Requirements

### Requirement: Encoder finite-q golomb luma DC block trace

The encoder SHALL provide a private finite-q golomb luma DC block trace stage
tracked by `ENC-INTRA-BLOCK-TRACE-GOLOMB-FINITE`, extending the coded DC block
trace on top of the §8.2.5 bypass-literal token. For a single luma DC coefficient
whose level reaches the §5.20.7.27 `maxLevel`, the luma `residual()` SHALL emit the
fixed level CDF tokens (`all_zero=0`, `eob_pt_16=0`, `coeff_base_eob=LF_NUM_BASE_LEVELS`,
`coeff_br=COEFF_BASE_RANGE`), then the §5.20.7.28 `read_quant` finite-q golomb
`coeff_rem` bypass bits encoding `x = magnitude - maxLevel` (for the first DC
coefficient `m=1`, so `q = x >> 1` `q_length_bit` zeros, a terminating `q_length_bit`
one, then `coeff_rem = x & 1`), then the luma `dc_sign` CDF token. The trace SHALL
compose the §5.20.5.3 mode-info prefix, that luma residual, then the all-zero U and
V `txb_skip`, and SHALL prove the complete trace writes through one in-tree AV2 §8.2
symbol encoder and decodes back through one symbol decoder with shared CDF state.
The proof SHALL include reconstructing the encoded magnitude from the decoded golomb
bits via the decoder's finite-q `read_quant` arithmetic. It SHALL NOT emit the
golomb-prefix tail (magnitude beyond the finite-q range), multi-coefficient blocks,
chroma golomb, partition syntax, tile payloads, coded packets, public CLI success,
or modes beyond the DC minimal tier.

#### Scenario: Golomb trace orders level tokens, coeff_rem bypass bits, then dc_sign

- **WHEN** the minimal finite-q golomb luma DC block trace is composed
- **THEN** the trace SHALL be exactly the ordered mode tokens, the fixed luma level
  tokens (`txb_skip=0`, `eob_pt_16`, `coeff_base_eob`, `coeff_br`), the golomb
  `q_length_bit` / `coeff_rem` bypass literals, the luma `dc_sign` token, then the
  all-zero U and V `txb_skip` tokens.

#### Scenario: Decoded golomb bits reconstruct the encoded magnitude

- **WHEN** the composed golomb trace is roundtripped through one in-tree AV2 §8.2
  coder and the decoded `coeff_rem` bypass bits are read back through the decoder's
  finite-q `read_quant` arithmetic
- **THEN** the reconstructed level SHALL equal the encoded coefficient magnitude
- **AND** the roundtrip SHALL be deterministic.

#### Scenario: Golomb trace does not produce packets

- **WHEN** the finite-q golomb block trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim the golomb-prefix tier,
  multi-coefficient syntax, or Baseline Encoder Profile v1 output from the trace
  alone.
