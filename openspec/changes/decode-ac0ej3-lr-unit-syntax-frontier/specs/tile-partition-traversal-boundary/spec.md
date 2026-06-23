## MODIFIED Requirements

### Requirement: Partition Traversal Frontier Boundary
The decoder SHALL provide a crate-private AV2 §5.20.3.1 partition traversal
frontier boundary tracked by Feature ID
`DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` and decoder support matrix row
`tile-partition-traversal-boundary`. The boundary SHALL compose the existing
partition-size table, allowed-partition derivation, partition decision,
partition-entry symbol read, and partition CDF context boundaries to advance
from a supported minimal intra tile partition root to the first
`decode_block()` frontier. When the caller supplies the narrow frame-level
Wiener NS loop-restoration state tracked by
`DECODE-AC0EJ3-LR-UNIT-SYNTAX-FRONTIER`, the boundary SHALL first consume the
covered AV2 §5.20.10.4/§5.20.10.5 `read_lr()` / `use_wiener_ns` symbols and
then continue partition traversal from the resulting live symbol cursor.

#### Scenario: Frontier uses existing partition components
- **WHEN** the frontier boundary reads a partition decision for a supported
  block on the path to the first `decode_block()` frontier
- **THEN** it derives allowed/implied facts through
  `tile-partition-allowed-boundary`
- **AND** it derives partition CDF selectors through
  `tile-cdf-selection-boundary`
- **AND** it consumes reached partition-entry symbols through
  `tile-partition-symbol-read-boundary`
- **AND** it resolves the final decision through
  `tile-partition-decision-boundary`
- **AND** it consumes supported frame-level Wiener NS LR unit symbols before the
  first partition-entry symbol when loop restoration is active for that narrow
  state

#### Scenario: Prefix child calls are emitted in spec order
- **WHEN** a supported frontier selects `PARTITION_HORZ`, `PARTITION_VERT`,
  `PARTITION_SPLIT`, `PARTITION_HORZ_3`, `PARTITION_VERT_3`,
  `PARTITION_HORZ_4A`, `PARTITION_HORZ_4B`, `PARTITION_VERT_4A`, or
  `PARTITION_VERT_4B`
- **THEN** the frontier result records the first child call in AV2 order with
  checked row, column, block-size, parent-size, chroma-offset, and has-chroma
  facts
- **AND** sibling children that cannot be processed until after the block
  frontier are retained as pending continuation metadata
- **AND** the block frontier carries a lossless §8.2 symbol-decoder checkpoint
  for the arithmetic state before block syntax begins

#### Scenario: Frontier stops at decode_block boundary
- **WHEN** traversal reaches `PARTITION_NONE`
- **THEN** it records a deterministic `decode_block()` frontier with row,
  column, block size, active tree/chroma facts, current syntax trace, and
  symbol-consumption counters
- **AND** it does not parse §5.20.4 block syntax, reconstruct pixels, update
  `MiSizes`, update references, emit output, call `exit_symbol()`, or claim
  runtime `decode_tile()` success

### Requirement: Traversal Scope Honesty
The traversal frontier boundary SHALL reject or defer unsupported paths with typed
crate-private errors and SHALL NOT expand public decode support beyond the
matrix row. SDP, BRU-active, bridge, broad inter/mixed-region behavior, broad
`decode_tile()`, unsupported loop-restoration variants, `decode_block()` syntax,
`MiSizes` mutation, reconstruction, output, CDF copyback/averaging,
block-decoder continuation APIs, and reference refresh behavior remain outside
this capability.

#### Scenario: Unsupported paths stay explicit
- **WHEN** traversal input requires SDP, PC-Wiener LR unit syntax, switchable LR
  unit syntax, per-unit Wiener coefficient syntax, BRU-active behavior, bridge
  behavior, broad inter-only behavior, or block syntax beyond partition
  traversal
- **THEN** the boundary returns an explicit unsupported/residual frontier result
  tied to `tile-partition-traversal-boundary`
- **AND** no public CLI success path or decoder support row is promoted by this
  capability

#### Scenario: Partition-step limits are separate from tile-count limits
- **WHEN** the frontier bounds the number of consumed partition decisions
- **THEN** it uses a dedicated partition-step decode limit
- **AND** it does not reuse the frame/tile-grid tile-count limit for recursive
  partition traversal

## ADDED Requirements

### Requirement: Frame-level Wiener NS LR Unit Syntax Frontier
The traversal frontier boundary SHALL model the ac0ej3 frame-level Wiener NS LR
unit syntax tracked by `DECODE-AC0EJ3-LR-UNIT-SYNTAX-FRONTIER`. For each covered
AV2 §5.20.10.4 restoration unit whose plane has
`FrameRestorationType == RESTORE_WIENER_NONSEP` and `frame_filters_on == true`,
the boundary SHALL consume one `use_wiener_ns S()` symbol from
`TileUseWienerNsCdf`, record/count the resulting LR type as either
`RESTORE_WIENER_NONSEP` or `RESTORE_NONE`, and skip the per-unit
`read_wienerns_filter(..., readFrameFilters == 0)` coefficient body exactly as
§5.20.10.6 specifies for frame-level filters.

#### Scenario: Frame-level Wiener NS units precede partition syntax
- **WHEN** a supported tile root covers one or more frame-level Wiener NS LR
  units before partition traversal
- **THEN** the traversal consumes the matching number of `use_wiener_ns` symbols
  before the first partition-entry symbol
- **AND** the resulting `decode_block()` frontier checkpoint reflects both the
  LR unit symbols and partition symbols already consumed

#### Scenario: Non-frame-level LR remains unsupported
- **WHEN** loop restoration uses PC-Wiener, switchable restoration, or Wiener NS
  without a frame-level bank for the active plane
- **THEN** the traversal rejects the input with a typed unsupported
  loop-restoration frontier before reading partition symbols
