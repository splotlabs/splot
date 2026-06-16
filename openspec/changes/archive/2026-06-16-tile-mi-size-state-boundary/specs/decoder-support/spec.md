## ADDED Requirements

### Requirement: Tile MI-size state boundary

The decoder support model SHALL track `DECODE-TILE-MI-SIZE-STATE-BOUNDARY` as a
crate-private `splot-decode` boundary for AV2 v1.0.0 § 5.20.4.1 MI-size state
updates over `MiSizes`, `LeftMiSizes`, and `AboveMiSizes`, with § 6.19.2.1
superblock-padded context extents.

#### Scenario: State initializes with clear-context sentinels
- **WHEN** the tile MI-size state boundary is constructed for finite frame MI
  dimensions
- **THEN** both luma and chroma `MiSizes` planes are initialized to the
  clear-context block-size sentinel used by the current minimal runtime frontier
  over superblock-padded row and column extents
- **AND** both luma and chroma `LeftMiSizes` and `AboveMiSizes` lines are
  initialized to the same sentinel over the corresponding padded extent
- **AND** zero dimensions or allocation arithmetic overflow fail with a typed
  crate-private error rather than panicking

#### Scenario: Runtime charges padded state before allocation
- **WHEN** the minimal runtime constructs tile MI-size state from parsed frame
  MI dimensions
- **THEN** it charges the superblock-padded grid cell count to `DecodeLimits`
  before allocation
- **AND** it charges the total MI-state `usize` entry storage bytes to
  `DecodeLimits` before allocation
- **AND** visible dimensions that fit a limit but require larger padded state
  fail with `decode/resource-limit` rather than allocating first

#### Scenario: Luma block update writes MI-size footprint
- **WHEN** a caller applies a checked luma block update with a validated AV2
  block size and in-frame `r`/`c` coordinates
- **THEN** every covered `MiSizes[0][r + y][c + x]` entry is set to that block
  size for the block's `Num_4x4_Blocks_High` by `Num_4x4_Blocks_Wide`
  footprint
- **AND** every covered `LeftMiSizes[0][r + y]` and `AboveMiSizes[0][c + x]`
  entry is set to that block size
- **AND** out-of-bounds or overflowing footprints fail before mutating state

#### Scenario: Luma edge block update may extend into padded context
- **WHEN** a caller applies a checked luma block update whose `r` and `c`
  coordinates are inside visible `MiRows` and `MiCols`
- **AND** the block's full footprint extends beyond visible `MiRows` or `MiCols`
  but remains inside the § 6.19.2.1 superblock-padded context extent
- **THEN** every covered padded `MiSizes[0]`, `LeftMiSizes[0]`, and
  `AboveMiSizes[0]` entry is set to that block size
- **AND** a block whose start coordinate is outside visible dimensions, or whose
  footprint exceeds the padded extent, fails before mutating state

#### Scenario: Chroma block update writes caller-supplied chroma footprint
- **WHEN** a caller applies a checked chroma block update with caller-supplied
  `ChromaMiRow`, `ChromaMiCol`, and `ChromaMiSize`
- **THEN** every covered `MiSizes[1][ChromaMiRow + y][ChromaMiCol + x]` entry
  is set to `ChromaMiSize`
- **AND** every covered `LeftMiSizes[1][ChromaMiRow + y]` and
  `AboveMiSizes[1][ChromaMiCol + x]` entry is set to `ChromaMiSize`
- **AND** out-of-bounds or overflowing chroma footprints fail before mutating
  state

#### Scenario: Existing partition context readers consume state views
- **WHEN** partition traversal or tests request partition-context state
- **THEN** the MI-size state boundary exposes read-only plane and neighbor-line
  views compatible with the existing `TilePartitionContextState` consumer
- **AND** those views reflect successful block updates
- **AND** this does not expose a public API or add scheduler ownership to
  `splot-recon`

#### Scenario: Broad tile decode remains partial
- **WHEN** decoder support and coverage documents are regenerated
- **THEN** `tile-payload-decode`, `tile-cdf-selection-boundary`,
  `intra-reconstruction`, runtime decode, and broad output rows remain partial
  or unsupported until separately implemented with runtime evidence
- **AND** this boundary does not claim full `decode_block()`, recursive
  `read_partition()`, broad `decode_tile()`, transform/residual parsing,
  reconstruction expansion, reference refresh, AVM/dav2d invocation, or
  external decoder integration
