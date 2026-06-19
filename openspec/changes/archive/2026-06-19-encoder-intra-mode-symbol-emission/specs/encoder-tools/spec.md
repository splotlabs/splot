## ADDED Requirements

### Requirement: Encoder intra-mode symbol emission minimal

The encoder SHALL provide a private intra-mode symbol-emission stage tracked by
`ENC-INTRA-MODE-SYMBOL-EMISSION`. For the current minimal subset, the stage SHALL
emit the ordered AV2 §5.20.5.5 `y_mode_set` and `y_mode_index` entropy-token
records for a DC_PRED luma block at the tile-origin neutral context, selecting the
§8.3.2 `TileYModeSetCdf` row (no context) and the `TileYModeIndexCdf` row at the
tile-origin context. The stage SHALL prove those token values can be written
through the in-tree AV2 §8.2 symbol encoder with scoped default CDF rows and
decoded back to the same values. It SHALL NOT emit chroma mode syntax,
coefficient or all-zero symbols, partition syntax, tile payloads, coded packets,
public CLI success, or broad intra-mode coverage beyond the declared minimal
tier.

#### Scenario: DC_PRED block emits ordered intra-mode tokens

- **WHEN** the minimal DC_PRED luma block at the tile origin is emitted
- **THEN** the stage SHALL report exactly the ordered `y_mode_set` and
  `y_mode_index` token records
- **AND** SHALL select the `y_mode_set` CDF row with no context and the
  `y_mode_index` CDF row at the tile-origin context, both with symbol value 0.

#### Scenario: Intra-mode tokens roundtrip through section 8.2 symbols

- **WHEN** the produced intra-mode token records are written through the in-tree
  AV2 section 8.2 symbol encoder using their scoped CDF rows
- **THEN** the bytes SHALL decode through the in-tree symbol decoder to the same
  ordered token values
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Unsupported intra-mode selectors are rejected

- **WHEN** an intra-mode CDF selector carries a `y_mode_index` context outside the
  supported range
- **THEN** the stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial token data.

#### Scenario: Intra-mode emission does not produce packets

- **WHEN** intra-mode symbol emission is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, chroma mode, coefficient, or CLI success from intra-mode emission alone.
