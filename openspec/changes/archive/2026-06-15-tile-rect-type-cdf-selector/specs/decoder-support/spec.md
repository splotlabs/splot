## MODIFIED Requirements

### Requirement: Tile CDF selection boundary

The decoder support model SHALL provide a crate-private tile CDF selection
boundary tracked by Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` and decoder
support matrix row `tile-cdf-selection-boundary`. The boundary SHALL copy a
small owned tile CDF subset from generated § 9.3 default tables, including the
partition-entry rows `DoSplitCdf`, `DoSquareSplitCdf`, `RectTypeCdf`,
`DoExtPartitionCdf`, and `DoUneven4wayPartitionCdf`; expose typed row selection
for § 8.3 `S` syntax-element handoff to `SymbolDecoder::read_symbol(cdf)`; and
record the § 8.2 frame-end CDF copy/average policy needed by a future
tile-completion row. The boundary SHALL NOT claim full § 8.3 CDF selection,
`rect_type` context derivation, full Tile/Saved CDF banks, recursive
`decode_tile()` / `decode_partition()` traversal, `exit_symbol()` after real
syntax, CDF copyback/averaging mutation after tile completion, reconstruction,
decoded-frame hashes, runtime Y4M output, reference refresh, public API support,
AVM/dav2d invocation, or new scheduler/dependency support.

#### Scenario: Default CDF subset is source-backed

- **WHEN** the tile CDF boundary initializes its owned frame/tile CDF subset
- **THEN** `DoSplitCdf`, `DoSquareSplitCdf`, `RectTypeCdf`,
  `DoExtPartitionCdf`, and `DoUneven4wayPartitionCdf` rows are copied from
  generated `splot-core` default CDF tables derived from AV2 § 9.3
- **AND** no CDF values are hand-transcribed from the spec mirror or a reference
  implementation

#### Scenario: Typed selectors bound CDF row access

- **WHEN** a caller requests a supported CDF row through a tile CDF selector
- **THEN** the boundary validates the selector contexts before indexing
- **AND** it returns mutable row access only for the duration of a caller
  closure suitable for `SymbolDecoder::read_symbol(cdf)`
- **AND** out-of-range selector contexts return typed errors without panicking or
  mutating CDF state

#### Scenario: Symbol decoder handoff honors CDF update policy

- **WHEN** a selected row is passed to `SymbolDecoder::read_symbol(cdf)`
- **THEN** the row is mutated when the tile work unit's CDF update mode is
  enabled
- **AND** the row remains byte-for-byte unchanged when
  `disable_cdf_update == 1` selects disabled CDF updates

#### Scenario: Copy and average policy is recorded only

- **WHEN** the boundary is asked for the frame-end CDF policy for a tile
- **THEN** it computes `copyCdf` and `avgCdf` from `enable_avg_cdf`,
  `avg_cdf_type`, `context_update_tile_id`, `TileNum`, and `TileCols * TileRows`
  according to AV2 § 8.2
- **AND** it does not apply Saved CDF mutation, CDF averaging, or
  `frame_end_update_cdf()` support until a future row wires real tile
  completion and `exit_symbol()`

#### Scenario: Runtime decode remains unsupported outside the boundary

- **WHEN** `splot decode` or the tile payload boundary reaches the CDF selection
  boundary after this change
- **THEN** it still reports structured `decode/unsupported-feature` metadata for
  the unimplemented `decode_tile()` / § 8.3 boundary
- **AND** it does not reconstruct pixels, compute hashes, write Y4M output,
  refresh references, locate or invoke external decoders, or bypass the
  `DecodeContext` worker-pool concurrency contract
