## ADDED Requirements

### Requirement: Encoder chroma uv_mode symbol emission

The encoder SHALL provide a private chroma `uv_mode` symbol-emission stage tracked
by `ENC-UV-MODE-SYMBOL-EMISSION`, implemented by extending the
`intra_mode_emission` module. For the current minimal subset, the stage SHALL emit
the ordered AV2 §5.20.5.6 `uv_mode` entropy-token record selecting the DC chroma
mode (`Default_Mode_List_Uv` index 0 = DC_PRED) for a non-directional DC_PRED luma
block, selecting the §8.3.2 `TileUVModeCflNotAllowedCdf` row at the
non-directional context 0. The stage SHALL prove that token value can be written
through the in-tree AV2 §8.2 symbol encoder with the scoped default CDF row and
decoded back to the same value. Per AV2 §5.20.5.3 `read_intra_uv_mode()` is called
after `read_intra_y_mode()` and before `residual()`, so `uv_mode` precedes all
coefficient symbols. This is valid only for a non-lossless block with CfL disabled
and MHCCP unavailable, where the §5.20.5.6 `use_dpcm_uv` and `is_cfl` predecessors
are not read. It SHALL NOT emit lossless `use_dpcm_uv` / `dpcm_mode_uv` or
`is_cfl` / CfL / CCTX / MHCCP syntax, coefficient or all-zero symbols, partition
syntax, tile payloads, coded packets, public CLI success, or chroma modes beyond
the declared DC minimal tier.

#### Scenario: DC chroma block emits the ordered uv_mode token

- **WHEN** the minimal DC chroma mode is emitted for a non-directional DC_PRED
  luma block
- **THEN** the stage SHALL report exactly the ordered `uv_mode` token record
- **AND** SHALL select the `TileUVModeCflNotAllowedCdf` row at the
  non-directional context 0 with symbol value 0.

#### Scenario: uv_mode token roundtrips through section 8.2 symbols

- **WHEN** the produced `uv_mode` token record is written through the in-tree AV2
  section 8.2 symbol encoder using its scoped CDF row
- **THEN** the bytes SHALL decode through the in-tree symbol decoder to the same
  token value
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Unsupported uv_mode selectors are rejected

- **WHEN** a `uv_mode` CDF selector carries a context outside the supported
  non-directional context
- **THEN** the stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial token data.

#### Scenario: uv_mode emission does not produce packets

- **WHEN** chroma `uv_mode` emission is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, coefficient, or CLI success from `uv_mode` emission alone.
