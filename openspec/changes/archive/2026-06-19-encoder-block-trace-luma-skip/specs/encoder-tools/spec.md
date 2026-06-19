## ADDED Requirements

### Requirement: Encoder unified block-symbol trace with luma txb_skip

The encoder SHALL provide a private unified block-symbol trace stage tracked by
`ENC-INTRA-BLOCK-TRACE-LUMA-SKIP`, extending the `block_symbol_trace` module with
a token kind spanning the intra-mode and coefficient token kinds. For the current
minimal subset, the stage SHALL compose the ordered AV2 trace `y_mode_set`,
`y_mode_index`, `uv_mode` (§5.20.5.3 mode info), then the luma `txb_skip`
all-zero token (§5.20.7.27, the first `residual()` symbol), and SHALL prove the
combined sequence writes through one in-tree AV2 §8.2 symbol encoder and decodes
back through one symbol decoder to the same ordered symbols with shared CDF
state, routing each token to its scoped §8.3.2 CDF row from `splot-core`
defaults. It SHALL NOT emit chroma `txb_skip`, non-all-zero luma coefficients,
partition syntax, tile payloads, coded packets, public CLI success, or modes
beyond the DC minimal tier.

#### Scenario: Composed trace is the mode prefix then luma txb_skip

- **WHEN** the minimal intra DC all-zero block trace is composed
- **THEN** the trace SHALL be exactly the ordered `y_mode_set`, `y_mode_index`,
  `uv_mode` mode tokens followed by the luma `txb_skip` all-zero token.

#### Scenario: Unified trace roundtrips through one section 8.2 coder

- **WHEN** the composed trace is written through one in-tree AV2 section 8.2
  symbol encoder using the scoped mode and `txb_skip` CDF rows
- **THEN** the bytes SHALL decode through one in-tree symbol decoder to the same
  ordered token symbols with shared CDF state
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Unsupported unified selectors are rejected

- **WHEN** the unified CDF router receives a token selector outside the supported
  minimal mode or luma `txb_skip` rows
- **THEN** the stage SHALL return a typed encoder error keyed by the token index
- **AND** SHALL NOT return partial roundtrip data.

#### Scenario: Unified trace does not produce packets

- **WHEN** the unified block-symbol trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, chroma coefficient syntax, or CLI success from the trace alone.
