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
matrix row. SDP, BRU-active, bridge, broad inter/mixed-region behavior, broad
`decode_tile()`, unsupported loop-restoration variants, `decode_block()` syntax,
`MiSizes` mutation, reconstruction, output, CDF copyback/averaging,
block-decoder continuation APIs, and reference refresh behavior remain outside
this capability.

#### Scenario: Unsupported paths stay explicit
- **WHEN** traversal input requires SDP, PC-Wiener LR unit syntax, switchable LR
  unit syntax, retaining or applying per-unit Wiener coefficients, BRU-active
  behavior, bridge behavior, broad inter-only behavior, or block syntax beyond
  partition traversal
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

### Requirement: Frame-level Wiener NS LR Unit Activity Summary
The tile partition traversal boundary SHALL report how many supported
frame-level Wiener NS LR units were consumed and how many selected
`RESTORE_WIENER_NONSEP` from the AV2 §5.20.10.5 `use_wiener_ns` symbol. A
`use_wiener_ns` value of zero SHALL be counted as an inactive `RESTORE_NONE`
unit, and a non-zero value SHALL be counted as an active
`RESTORE_WIENER_NONSEP` unit. The boundary SHALL preserve the existing
transactional CDF behavior: failed traversal attempts MUST NOT commit LR-unit
CDF mutations.

#### Scenario: Inactive frame-level units are reported
- **WHEN** a supported superblock-root LR frontier consumes frame-level Wiener
  NS units whose `use_wiener_ns` symbols all select zero
- **THEN** the frontier reports the consumed unit count
- **AND** it reports zero active Wiener NS units
- **AND** it commits the same CDF updates and symbol position as the existing LR
  syntax frontier

#### Scenario: Active frame-level units are reported
- **WHEN** a supported superblock-root LR frontier consumes a frame-level Wiener
  NS unit whose `use_wiener_ns` symbol selects non-zero
- **THEN** the frontier reports at least one active Wiener NS unit
- **AND** callers can fail closed before claiming loop-restoration
  reconstruction or output support

#### Scenario: Rejected LR paths stay transactional
- **WHEN** the LR frontier fails due to a resource limit, unsupported SDP plane
  range, unsupported LR variant, or invalid unit geometry
- **THEN** the work unit's tile CDF subset remains unchanged
- **AND** no inactive-or-active LR-unit support claim is made for that input

### Requirement: Active Wiener NS LR Source-Bounds Frontier

The tile partition traversal boundary SHALL retain active frame-level Wiener NS
loop-restoration source-bound facts for the supported root LR frontier.
For each retained block, the facts SHALL identify the plane, luma 4x4 row and
column, selected LR unit row and column, current-plane block coordinates and
size, and the caller-resolved AV2 §7.20.1 luma source/stripe bounds. Failed
source-bound derivation MUST NOT commit LR-unit CDF mutations.

When an active Wiener NS LR unit reaches AV2 §5.20.10.6 with
`readFrameFilters == 0`, the boundary SHALL consume the entropy-coded per-unit
Wiener NS filter syntax needed to complete `read_lr()` before retaining the
source-bound facts. The decoded coefficients SHALL NOT be exposed as
reconstruction support by this boundary.

#### Scenario: Active source bounds are retained for a supported root unit

- **WHEN** a supported root LR frontier consumes an active
  frame-level Wiener NS unit
- **THEN** the frontier includes active source-bound facts for the covered
  loop-restore blocks
- **AND** each retained block cites the active unit row and column selected by
  the already-consumed LR unit syntax
- **AND** each retained block includes the §7.20.1 luma source and stripe bounds

#### Scenario: Per-unit Wiener NS filter syntax completes before bounds

- **WHEN** an active Wiener NS unit uses `readFrameFilters == 0`
- **THEN** the boundary consumes the required §5.20.10.6 per-unit filter syntax
- **AND** source-bound facts are retained only after that syntax succeeds
- **AND** the decoded filter coefficients are not reported as reconstruction
  output

#### Scenario: Inactive units do not retain active source blocks

- **WHEN** a supported root LR frontier consumes only inactive frame-level
  Wiener NS units
- **THEN** the frontier preserves the inactive unit selections
- **AND** the active source-bound list is empty

#### Scenario: Tile-clamped source bounds follow the sequence filter flag

- **WHEN** an active LR unit is consumed for a tile range smaller than the frame
- **AND** loop filters are disabled across tiles
- **THEN** retained source-bound facts use the tile MI range for `LumaStart*`
  and `LumaEnd*`

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

### Requirement: Frame-Level Wiener NS LR Unit Selection State

The tile partition traversal boundary SHALL preserve the supported frame-level
Wiener NS LR-unit selections in syntax order. Each selection SHALL identify the
plane, the absolute LR unit row and column after tile-origin offset adjustment,
and whether AV2 §5.20.10.5 `use_wiener_ns` selected active
`RESTORE_WIENER_NONSEP`. The boundary SHALL preserve existing aggregate consumed
and active unit counts, and failed traversal attempts MUST NOT commit LR-unit CDF
mutations.

#### Scenario: Inactive unit selection is retained

- **WHEN** a supported superblock-root LR frontier consumes an inactive
  frame-level Wiener NS unit
- **THEN** the frontier includes one selection with the corresponding plane,
  unit row, unit column, and `active = false`
- **AND** the aggregate active count remains zero

#### Scenario: Active unit selection is retained

- **WHEN** a supported superblock-root LR frontier consumes an active
  frame-level Wiener NS unit
- **THEN** the frontier includes one selection with the corresponding plane,
  unit row, unit column, and `active = true`
- **AND** callers can continue to fail closed before claiming loop-restoration
  reconstruction or output support

#### Scenario: Multi-unit syntax order is retained

- **WHEN** a supported superblock-root LR frontier covers multiple frame-level
  Wiener NS LR units
- **THEN** the frontier's selections are ordered by the §5.20.10.4 unit-row loop
  and then the unit-column loop
- **AND** each stored coordinate uses the tile-origin-adjusted LR unit index

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

#### Scenario: SDP LR plane ranges remain unsupported
- **WHEN** an intra SDP root would require frame-level Wiener NS LR symbols for
  a non-luma `PlaneStart..PlaneEnd` range
- **THEN** the traversal rejects the SDP path before reading LR unit symbols

#### Scenario: Non-frame-level LR remains unsupported
- **WHEN** loop restoration uses PC-Wiener, switchable restoration, or Wiener NS
  without a frame-level bank for the active plane
- **THEN** the traversal rejects the input with a typed unsupported
  loop-restoration frontier before reading partition symbols
