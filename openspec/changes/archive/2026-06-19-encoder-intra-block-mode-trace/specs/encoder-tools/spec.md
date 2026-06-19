## ADDED Requirements

### Requirement: Encoder intra-block mode trace composition

The encoder SHALL provide a private intra-block mode-trace composition stage
tracked by `ENC-INTRA-BLOCK-MODE-TRACE`, in a `block_symbol_trace` module. For
the current minimal subset, the stage SHALL compose the ordered AV2 §5.20.5.3
mode-info prefix — `y_mode_set`, `y_mode_index`, then `uv_mode` — by reusing the
merged luma and chroma mode emitters, and SHALL prove the composed sequence
writes through one in-tree AV2 §8.2 symbol encoder and decodes back through one
symbol decoder to the same ordered symbols with shared CDF state. It SHALL NOT
emit coefficient or all-zero symbols, partition syntax, tile payloads, coded
packets, public CLI success, or modes beyond the DC minimal tier.

#### Scenario: Composed trace is the ordered mode-info prefix

- **WHEN** the minimal intra DC block mode trace is composed
- **THEN** the trace SHALL be exactly the ordered luma `y_mode_set` and
  `y_mode_index` tokens followed by the chroma `uv_mode` token.

#### Scenario: Composed trace roundtrips through one section 8.2 coder

- **WHEN** the composed trace is written through one in-tree AV2 section 8.2
  symbol encoder using the scoped CDF rows
- **THEN** the bytes SHALL decode through one in-tree symbol decoder to the same
  ordered token symbols with shared CDF state
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Mode trace does not produce packets

- **WHEN** the intra-block mode trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, coefficient syntax, or CLI success from the mode trace alone.
