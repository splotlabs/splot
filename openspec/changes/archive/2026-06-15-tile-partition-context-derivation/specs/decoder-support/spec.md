## MODIFIED Requirements

### Requirement: Tile CDF selection boundary

The decoder support model SHALL provide a crate-private tile CDF selection
boundary tracked by Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` and decoder
support matrix row `tile-cdf-selection-boundary`. The boundary SHALL copy a
small owned tile CDF subset from generated § 9.3 default tables, including the
partition-entry rows `DoSplitCdf`, `DoSquareSplitCdf`, `RectTypeCdf`,
`DoExtPartitionCdf`, and `DoUneven4wayPartitionCdf`; expose typed row selection
for § 8.3 `S` syntax-element handoff to `SymbolDecoder::read_symbol(cdf)`;
derive bounded left/above-neighbor § 8.3.2 contexts for `do_split`,
`rect_type`, `do_ext_partition`, and `do_uneven_4way_partition`; and record the
§ 8.2 frame-end CDF copy/average policy needed by a future tile-completion row.
The boundary SHALL NOT claim `do_square_split` context derivation, full § 8.3
CDF selection, full Tile/Saved CDF banks, recursive `decode_tile()` /
`read_partition()` traversal, `exit_symbol()` after real syntax, CDF
copyback/averaging mutation after tile completion, reconstruction,
decoded-frame hashes, runtime Y4M output, reference refresh, public API support,
AVM/dav2d invocation, or new scheduler/dependency support.

#### Scenario: Left and above partition contexts are bounded

- **WHEN** the tile CDF boundary derives contexts for `do_split`, `rect_type`,
  `do_ext_partition`, or `do_uneven_4way_partition`
- **THEN** every `bSize`, `PlaneStart`, `r`, `c`, second-half extended
  partition offset, and neighbor block-size lookup is bounds-checked before use
- **AND** invalid indexes return crate-private typed errors instead of panicking
- **AND** the resulting context is checked against the selected CDF array before
  row access

#### Scenario: Partition context derivation remains partial

- **WHEN** decoder support status is rendered
- **THEN** the tile CDF selection boundary still reports partial status
- **AND** `do_square_split` context derivation, actual syntax reads,
  `read_partition()`, and `decode_tile()` remain out of scope
