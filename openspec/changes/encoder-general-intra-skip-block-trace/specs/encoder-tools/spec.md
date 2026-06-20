## ADDED Requirements

### Requirement: Encoder general-intra DC skip-block trace

The encoder block-symbol trace SHALL compose the complete ordered AV2 general intra decode
symbol stream for one undivided 64x64 DC skip (all-zero) superblock, tracked by
`ENC-GENERAL-INTRA-SKIP-BLOCK-TRACE`: the § 5.20.3.2 `do_split == false` flag, the § 5.20.5.3
mode-info prefix (`y_mode_set`, `y_mode_index`, `uv_mode`), then the per-plane § 5.20.7.27
`all_zero` (`txb_skip`) symbols for luma, U, and V. It SHALL code the luma and U `txb_skip`
symbols at the general 64x64-leaf transform contexts (`TX_64X64` luma, `TX_32X32` chroma) and
compose through the existing block-symbol-trace § 8.2 coder. It SHALL NOT claim a `tile_data`
payload, a tile, a frame, a packet, or `Context::receive_packet` output.

#### Scenario: The general skip-block trace round-trips

- **WHEN** the general intra DC skip-block trace is composed
- **THEN** it SHALL be the ordered seven-symbol trace `[do_split, y_mode_set, y_mode_index,
  uv_mode, luma all_zero, U all_zero, V all_zero]` with symbols `[0, 0, 0, 0, 1, 1, 1]`
- **AND** round-tripping it through one § 8.2 coder SHALL decode to `[0, 0, 0, 0, 1, 1, 1]`
  with `symbol_count == 7`.

#### Scenario: The general txb_skip tokens target the 64x64-leaf transform contexts

- **WHEN** the general luma and U `all_zero` tokens are emitted
- **THEN** the luma token SHALL select the `TX_64X64` `txb_skip` row (`txSzCtx 4`, `ctx 0`)
- **AND** the U token SHALL select the `TX_32X32` `txb_skip` row (`txSzCtx 3`, `ctx 6`).

#### Scenario: The bridge does not produce packets

- **WHEN** the general skip-block composer is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a tile, a frame, a packet, or Baseline
  Encoder Profile v1 output from it.
