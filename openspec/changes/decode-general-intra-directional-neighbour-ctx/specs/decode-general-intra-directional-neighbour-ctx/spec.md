## ADDED Requirements

### Requirement: General intra directional-neighbour (ctx != 0) y_mode_index decode
The decoder SHALL decode the AV2 § 5.20.5.3 `read_intra_y_mode` mode symbols of a
general intra block whose AV2 § 8.3.2 `y_mode_index` context is non-zero — i.e.
whose already-decoded left or above neighbour stored a directional `IntraJointMode`
(`>= NON_DIRECTIONAL_MODES_COUNT`) — by reading `y_mode_set` / `y_mode_index` from
the real `TileYModeIndexCdf[ctx]` row (ctx 1 or 2, from the per-MI `IntraJointModes`
grid), instead of rejecting before the read. For a block whose decoded
`y_mode_index` is non-directional (`modeIdx < NON_DIRECTIONAL_MODES_COUNT`, not the
`MODE_INDEX_COUNT - 1` `y_mode_offset` escape), the decoder SHALL resolve the typed
luma `YMode` through § 5.20.5.3 `get_intra_y_mode_set`, which returns `modeIdx`
unchanged regardless of neighbours (`05-syntax-structures.md` lines 11116-11118), so
the directional-neighbour selection loop is not entered; the reconstructed mode SHALL
be `Reordered_Y_Mode[modeIdx]`.

The decoder SHALL defer — with a structured `decode/unsupported-feature` diagnostic,
after the `y_mode_offset` symbol is consumed — the § 5.20.5.3 `y_mode_offset` escape
(`y_mode_set == 0`, `y_mode_index == MODE_INDEX_COUNT - 1`) when the block has a
directional joint-mode neighbour (`ctx != 0`), because the escape's
`modeIdx >= NON_DIRECTIONAL_MODES_COUNT` enters the directional-neighbour reorder of
`get_intra_y_mode_set` (lines 11120-11176), which the no-directional-neighbour escape
reconstruction does not model and which resolves to a directional mode needing the
deferred § 7.13.2.8 luma IDIF. The decoder SHALL continue to reject — with a
structured `decode/unsupported-feature` diagnostic — a directional neighbour-reading
LUMA block over a real reconstructed (non-flat) neighbour edge, the
`y_mode_set != 0` / `y_second_mode` path, sub-superblock non-DC blocks, multiple
tiles, inter prediction, and in-loop filters, and SHALL NOT invoke AVM or dav2d.

#### Scenario: A SMOOTH_V block with a D135 left neighbour decodes to the oracle
- **WHEN** `splot decode` is given the committed multi-superblock intra key frame
  `syn-dirneigh-intra-128x64-q80.ivf`, whose LEFT 64x64 superblock codes as
  `D135_PRED` luma (storing the directional `IntraJointMode 36`) and whose RIGHT
  64x64 superblock codes as `SMOOTH_V_PRED` luma
- **THEN** the RIGHT block's § 8.3.2 `y_mode_index` context resolves to 1 (the
  directional left neighbour), and the decoder reads `y_mode_set` / `y_mode_index`
  from `TileYModeIndexCdf[1]` and reconstructs `SMOOTH_V_PRED` over the real
  reconstructed left-neighbour edge, succeeding
- **AND** the decoded output matches the avmdec and dav2d raw outputs byte-for-byte
  (md5 `1a84b6545ee333b98cdf1982fd18310a`)
- **AND** the decoded-frame hash is the pinned
  `ad1515885df5620a31c37f855934ae2432167edbf1b1b62081552b9df3957426`

#### Scenario: The old code rejected this exact frame
- **WHEN** the same `syn-dirneigh-intra-128x64-q80.ivf` frame is decoded
- **THEN** the RIGHT block's non-zero `y_mode_index` context (ctx 1, from the D135
  left neighbour's `IntraJointMode 36 >= NON_DIRECTIONAL_MODES_COUNT`) is the case
  that the previous brick rejected with the
  `general_intra_directional_neighbour_y_mode_index_ctx` diagnostic, so the brick
  decodes a frame that did not previously decode

#### Scenario: A directional neighbour-reading luma block is still deferred
- **WHEN** a general intra block with a directional joint-mode neighbour decodes a
  DIRECTIONAL luma mode over a real reconstructed (non-flat) neighbour edge (whether
  via the non-escape index or the `y_mode_offset` escape over the directional
  neighbour)
- **THEN** the decoder emits a structured `decode/unsupported-feature` diagnostic
  (the real § 7.13.2.8 luma IDIF, and the § 5.20.5.3 in-frame directional-neighbour
  reorder, are not yet modelled), rather than reconstructing an unverified prediction

#### Scenario: Existing general intra fixtures still decode bit-exact
- **WHEN** `splot decode` is given the committed 64x64, 128x64, 64x128, 128x128, and
  192x128 general intra fixtures
- **THEN** each reconstructs to its previously pinned decoded-frame hash, unchanged
  by lifting the `ctx != 0` `y_mode_index` reject
