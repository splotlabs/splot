# tile-partition-traversal-boundary Specification

## Purpose
Define the crate-private AV2 tile partition traversal frontier that composes
existing partition helpers up to, but not beyond, the first `decode_block()`
boundary.
## Requirements
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
- **AND** it gates §5.20.10.4 root `read_lr()` syntax with explicit typed
  unsupported state before any partition-entry symbol is read

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
- **WHEN** traversal input requires SDP, §5.20.10.4 root `read_lr()` syntax,
  BRU-active behavior, bridge behavior, inter-only behavior, or block syntax
  beyond partition traversal
- **THEN** the boundary returns an explicit unsupported/residual frontier result
  tied to `tile-partition-traversal-boundary`
- **AND** no public CLI success path or decoder support row is promoted by this
  capability

#### Scenario: Partition-step limits are separate from tile-count limits
- **WHEN** the frontier bounds the number of consumed partition decisions
- **THEN** it uses a dedicated partition-step decode limit
- **AND** it does not reuse the frame/tile-grid tile-count limit for recursive
  partition traversal

### Requirement: Runtime Live Cursor Frontier Bridge
The tile partition traversal boundary SHALL provide a crate-private bridge that
can return both the existing traversal plan and the live §8.2 symbol decoder
cursor after the first `decode_block()` frontier. The bridge SHALL be usable by
the minimal runtime without adding a public `splot-core` checkpoint-resume API
or expanding the frontier beyond §5.20.3.1 partition traversal.

#### Scenario: Live cursor matches frontier checkpoint
- **WHEN** the runtime bridge reaches the root `decode_block()` frontier for the
  committed minimal tile payload
- **THEN** the returned traversal plan records the same symbol count and
  consumed-bit position as the live symbol decoder cursor
- **AND** the live cursor can continue decoding the existing traced flat-block
  symbols without replaying the root partition symbol manually

#### Scenario: Bridge remains narrower than decode block
- **WHEN** the root partition frontier is planned for the minimal runtime
- **THEN** the bridge asserts the frontier before §5.20.4.1 `decode_block()`
- **AND** it does not mutate `MiSizes`, parse block syntax, reconstruct pixels,
  update references, emit output, or perform CDF copyback/averaging

### Requirement: Active Wiener NS LR Source-Read Frontier
The tile partition traversal boundary and minimal runtime SHALL advance active
frame-level Wiener NS loop-restoration source-bound facts to a fail-closed
source-read frontier for `DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER`. The frontier
SHALL use caller-resolved AV2 §7.20.1 bounds to attempt AV2 §7.20.2 source
sample selection state for supported active block centers, and MUST NOT claim
complete Wiener tap reads, chroma luma-source reads, §7.20.3 filtering, or
decoded output.

#### Scenario: Active source reads are attempted after bounds
- **WHEN** a supported root LR frontier retains active Wiener NS source-bound
  facts
- **THEN** the runtime attempts source sample selection state for active
  block-center coordinates
- **AND** the previous source-bounds diagnostic is no longer the live ac0ej3
  frontier

#### Scenario: Source reads remain fail-closed before filtering
- **WHEN** the active source-read boundary is reached for the local ac0ej3
  mission stream
- **THEN** the runtime emits a structured unsupported diagnostic for the
  source-read/filtering frontier
- **AND** no complete Wiener tap reads, chroma luma-source reads, §7.20.3 Wiener
  NS filtering, decoded-frame allocation, reference refresh, hash, raw, or Y4M
  output is produced

#### Scenario: Source-read failures are transactional
- **WHEN** source sample selection derivation fails for an active LR block
- **THEN** the runtime reports a structured decode error or unsupported
  diagnostic
- **AND** LR CDF mutations and retained frontier state are not committed past
  the failed read boundary
