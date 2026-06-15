# tile-partition-size-tables Specification

## Purpose
Define the crate-private decoder boundary for AV2 § 9.2 partition-size table
lookups, including generated table backing, `BLOCK_INVALID` preservation, and
the scope limits that keep this narrower than recursive tile partition
traversal.
## Requirements
### Requirement: Tile partition size table boundary

The decoder SHALL provide a crate-private AV2 § 9.2 partition-size lookup
boundary tracked by Feature ID `DECODE-TILE-PARTITION-SIZE-TABLE-BOUNDARY` and
decoder support matrix row `tile-partition-size-tables`. The boundary SHALL
expose bounded helpers for `Partition_Subsize[partition][bSize]` and
`H_Partition_Midsize[bSize]` backed by generated `splot-core` § 9.2 conversion
tables, SHALL preserve `BLOCK_INVALID` as an explicit typed result, SHALL return
valid sub-block sizes only when the table entry is a valid AV2 block size, and
SHALL NOT claim recursive partition traversal, allowed-partition derivation,
reconstruction, output, public API behavior, or external decoder invocation.

#### Scenario: Valid partition subsize is returned
- **WHEN** crate-private decoder code requests a valid `Partition_Subsize`
  combination whose AV2 § 9.2 table entry is a valid `BLOCK_*` value
- **THEN** the helper returns the corresponding typed valid block size
- **AND** the returned value can be used as a checked block-size index by future
  traversal code

#### Scenario: Invalid partition subsize is preserved
- **WHEN** crate-private decoder code requests a valid `Partition_Subsize`
  combination whose AV2 § 9.2 table entry is `BLOCK_INVALID`
- **THEN** the helper returns an explicit invalid result
- **AND** it does not map `BLOCK_INVALID` to a valid block-size index

#### Scenario: Horizontal midsize lookup is bounded
- **WHEN** crate-private decoder code requests `H_Partition_Midsize[bSize]`
  for a valid AV2 block size
- **THEN** the helper returns the corresponding typed valid block size or
  explicit invalid result from the § 9.2 table

#### Scenario: Out-of-range table inputs are rejected
- **WHEN** a caller tries to construct or look up an out-of-range block-size
  index
- **THEN** the boundary returns a typed crate-private error before indexing the
  table
- **AND** the error identifies the table, supplied index, and maximum exclusive
  bound

#### Scenario: Scope remains narrower than partition traversal
- **WHEN** decoder support status is generated
- **THEN** `tile-partition-size-tables` records support only for the
  crate-private § 9.2 table lookup boundary
- **AND** `tile-payload-decode` remains partial for `partition_implied`,
  `init_allowed_partitions`, full allowed-partition derivation, recursive
  `read_partition()`/`decode_partition()` traversal, `decode_tile()`,
  reconstruction, output, reference refresh, and public runtime decode
