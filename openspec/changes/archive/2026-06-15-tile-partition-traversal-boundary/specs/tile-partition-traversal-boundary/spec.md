## ADDED Requirements

### Requirement: Partition Traversal Frontier Boundary
The decoder SHALL provide a crate-private AV2 §5.20.3.1 partition traversal
frontier boundary tracked by Feature ID
`DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` and decoder support matrix row
`tile-partition-traversal-boundary`. The boundary SHALL compose the existing
partition-size table, allowed-partition derivation, partition decision,
partition-entry symbol read, and partition CDF context boundaries to advance
from a supported minimal intra tile partition root to the first
`decode_block()` frontier.

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

### Requirement: Transactional Partition Context Reads
The traversal frontier boundary SHALL use caller-provided bounded block-size
context state required by AV2 §8.3.2 partition context derivation and SHALL NOT
mutate that state. The boundary SHALL clone mutable tile CDF rows and initialize
a fresh symbol decoder from the tile work unit, then commit CDF row mutations
back to the work unit only after a frontier is planned successfully.

#### Scenario: Failed frontier leaves work unit unchanged
- **WHEN** partition CDF context derivation or symbol decoding fails before a
  frontier is planned
- **THEN** the tile work unit's CDF rows remain unchanged

#### Scenario: Disabled CDF update remains immutable
- **WHEN** the tile work unit's CDF update mode is disabled
- **THEN** the frontier can advance symbol state without mutating CDF rows

#### Scenario: Context state bounds are checked
- **WHEN** traversal context state is created or read
- **THEN** row, column, and plane accesses are checked
- **AND** coordinate arithmetic failures return typed frontier errors instead
  of panicking or wrapping

### Requirement: Traversal Scope Honesty
The traversal frontier boundary SHALL reject or defer unsupported paths with typed
crate-private errors and SHALL NOT expand public decode support beyond the
matrix row. SDP, BRU-active, bridge, inter/mixed-region, broad `decode_tile()`,
`decode_block()` syntax, `MiSizes` mutation, reconstruction, output, CDF
copyback/averaging, block-decoder continuation APIs, and reference refresh
behavior remain outside this capability.

#### Scenario: Unsupported paths stay explicit
- **WHEN** traversal input requires SDP, BRU-active behavior, bridge behavior,
  inter-only behavior, or block syntax beyond partition traversal
- **THEN** the boundary returns an explicit unsupported/residual frontier result
  tied to `tile-partition-traversal-boundary`
- **AND** no public CLI success path or decoder support row is promoted by this
  capability
