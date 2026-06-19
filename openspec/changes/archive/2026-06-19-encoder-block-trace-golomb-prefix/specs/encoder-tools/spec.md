## ADDED Requirements

### Requirement: Encoder golomb-prefix luma DC block trace

The encoder SHALL provide a private golomb-prefix luma DC block trace stage tracked
by `ENC-INTRA-BLOCK-TRACE-GOLOMB-PREFIX`, extending the finite-q golomb tail to the
§5.20.7.28 `read_quant` golomb-prefix path (`q == cMax`) for a single luma DC
coefficient of magnitude 18..=525. The luma `residual()` SHALL emit the fixed level
CDF tokens, then the luma `dc_sign` CDF token, then the golomb-prefix bypass bits:
`cMax` (5) `q_length` zeros, the `golomb_length` unary (`golomb_zeros` zeros and a
terminating 1, where `length = golomb_zeros + k` and `k = 2`), and `coeff_rem` as
one `L(length)` literal, encoding `x = magnitude - maxLevel` with
`length = GetMsb(x - 6)`, `golomb_zeros = length - k`, and
`coeff_rem = (x - 6) - (1 << length)`. The trace SHALL compose the §5.20.5.3
mode-info prefix, that luma residual, then the all-zero U and V `txb_skip`, and
SHALL prove the complete trace writes through one in-tree AV2 §8.2 symbol encoder
and decodes back through one symbol decoder with shared CDF state. The proof SHALL
include reconstructing each encoded magnitude from the decoded golomb-prefix bits
via the decoder's golomb-prefix `read_quant` arithmetic. Magnitudes outside
18..=525 SHALL be rejected with a typed error at runtime. It SHALL NOT emit
magnitudes beyond 525, multi-coefficient blocks, chroma golomb, partition syntax,
tile payloads, coded packets, public CLI success, or modes beyond the DC minimal
tier.

#### Scenario: Golomb-prefix trace orders level tokens, dc_sign, then prefix bypass bits

- **WHEN** the minimal golomb-prefix luma DC block trace (magnitude 18) is composed
- **THEN** the trace SHALL be exactly the ordered mode tokens, the fixed luma level
  tokens (`txb_skip=0`, `eob_pt_16`, `coeff_base_eob`, `coeff_br`), the luma
  `dc_sign` token, the five `q_length` bypass zeros, the `golomb_length` unary
  bypass bits, the `coeff_rem` `L(length)` bypass literal, then the all-zero U and V
  `txb_skip` tokens.

#### Scenario: Decoded golomb-prefix bits reconstruct the encoded magnitude across the range

- **WHEN** a golomb-prefix trace is composed for every magnitude in the supported
  range (18..=525), roundtripped through one in-tree AV2 §8.2 coder, and the decoded
  bypass bits are read back through the decoder's golomb-prefix `read_quant`
  arithmetic
- **THEN** the reconstructed level SHALL equal each encoded coefficient magnitude
- **AND** the roundtrip SHALL be deterministic.

#### Scenario: Out-of-range golomb-prefix magnitude is rejected

- **WHEN** the golomb-prefix compose is called with a magnitude outside 18..=525
- **THEN** it SHALL return a typed `BlockSymbolTraceGolombMagnitudeOutOfRange`
  error at runtime (not via a release-stripped debug assertion)
- **AND** SHALL NOT emit a trace for the non-conformant coefficient.

#### Scenario: Golomb-prefix trace does not produce packets

- **WHEN** the golomb-prefix block trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim multi-coefficient syntax or
  Baseline Encoder Profile v1 output from the trace alone.
