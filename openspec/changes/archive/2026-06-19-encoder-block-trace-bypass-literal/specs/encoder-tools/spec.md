## ADDED Requirements

### Requirement: Encoder block-symbol bypass-literal token

The encoder SHALL provide a private bypass-literal block-symbol token tracked by
`ENC-INTRA-BLOCK-TRACE-BYPASS-LITERAL`, extending the unified block-symbol trace.
The token SHALL represent an AV2 §8.2.5 `L(n)` bypass literal of a given bit width
and value (the foundation for the `sign_bit` of a chroma or ordinary non-axis luma coefficient per §5.20.7.27 — the luma DC sign is `dc_sign` and the directional luma axis signs are `dc_sign_horz_vert`, both CDF
and the §5.20.7.28 golomb tail). The `roundtrip_block_symbol_trace` proof SHALL
write a bypass token through the in-tree `SymbolEncoder` literal primitive and read
it back through the `SymbolDecoder` literal primitive, interleaved with CDF
symbols, in one §8.2 coder. It SHALL NOT, by itself, emit coded chroma signs, the
golomb tail, tile payloads, coded packets, public CLI success, or modes beyond the
current trace.

#### Scenario: Bypass literals interleave with CDF symbols

- **WHEN** a block-symbol trace mixes CDF-coded tokens and bypass-literal tokens
- **THEN** writing it through one in-tree AV2 §8.2 coder and reading it back SHALL
  reproduce the same ordered values (the CDF symbols and the literal values)
- **AND** the roundtrip SHALL be deterministic.

#### Scenario: Bypass literal carries no CDF row

- **WHEN** a bypass-literal token is written or read in the trace roundtrip
- **THEN** it SHALL use the `SymbolEncoder`/`SymbolDecoder` literal primitives with
  no CDF-row selection
- **AND** it SHALL NOT consume or mutate any CDF row.

#### Scenario: Bypass-literal foundation does not produce packets

- **WHEN** the bypass-literal token kind is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim coded chroma signs, the golomb
  tail, or Baseline Encoder Profile v1 output from the token kind alone.
