## ADDED Requirements

### Requirement: Tile partition allowed boundary

The decoder SHALL provide a crate-private AV2 § 5.20.3.2 partition implied and
allowed-partition derivation boundary tracked by Feature ID
`DECODE-TILE-PARTITION-ALLOWED-BOUNDARY` and decoder support matrix row
`tile-partition-allowed-boundary`. The boundary SHALL derive
`partition_implied`, `partition_implied_at_boundary`,
`rect_type_implied_by_bsize`, `is_partition_allowed`, and
`init_allowed_partitions` from explicit caller-provided frame/tile facts; SHALL
reuse the existing typed partition and block-size boundaries; SHALL preserve
`BLOCK_INVALID` as an invalid result; and SHALL NOT claim recursive
partition traversal, symbol reads, CDF mutation, reconstruction, output, public
API behavior, or external decoder invocation.

#### Scenario: Boundary implication is derived
- **WHEN** crate-private decoder code supplies bounded MI frame dimensions,
  block origin, tree type, and block size facts
- **THEN** the boundary returns the same implied-partition result as AV2
  `partition_implied_at_boundary`
- **AND** it handles right and bottom frame-edge cases without panicking

#### Scenario: Direct implication rules are derived
- **WHEN** the block size or tree type matches the direct AV2
  `partition_implied` rules
- **THEN** the boundary returns the required implied partition without reading
  partition symbols
- **AND** the chroma `BLOCK_64X64` luma-partition reuse rule is driven only by
  an explicit caller-provided known-luma-partition fact

#### Scenario: Allowed set is derived
- **WHEN** crate-private decoder code supplies partition geometry, tree/chroma
  facts, feature flags, and region facts
- **THEN** the boundary evaluates every `PartitionType` with
  `is_partition_allowed`
- **AND** returns an `AllowedPartitions` set and count in AV2 partition order

#### Scenario: Empty allowed set falls back to none
- **WHEN** every candidate partition is disallowed by the supplied facts
- **THEN** the boundary returns an allowed set containing only
  `PARTITION_NONE`
- **AND** reports a count of one, matching AV2 `init_allowed_partitions`

#### Scenario: Invalid subsize and residual-size entries are rejected
- **WHEN** `Partition_Subsize[p][bSize]` or `get_plane_residual_size(...)`
  produces `BLOCK_INVALID`
- **THEN** the candidate partition is disallowed rather than mapping the
  sentinel to a valid block-size index

#### Scenario: Geometry errors are typed
- **WHEN** caller-provided MI coordinates or derived partition offsets overflow
  checked arithmetic
- **THEN** the boundary returns a crate-private typed error
- **AND** no symbol decoder state, CDF rows, `MiSizes`, output files, or public
  diagnostics are changed

#### Scenario: Scope remains narrower than traversal
- **WHEN** decoder support status is generated
- **THEN** `tile-partition-allowed-boundary` records support only for
  crate-private implied and allowed-partition derivation
- **AND** `tile-payload-decode` remains partial for recursive
  `read_partition()`/`decode_partition()` traversal, `decode_tile()`,
  `MiSizes` mutation, reconstruction, output, reference refresh, and public
  runtime decode
