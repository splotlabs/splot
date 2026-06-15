## Why

AV2 § 5.20.3.2 `read_partition()` still receives caller-provided
implied-partition and allowed-partition facts in `splot-decode`. The partition
decision and size-table boundaries are now in place, so the next narrow
decoder step is to derive those facts from bounded frame/tile geometry instead
of hand-written test inputs.

## What Changes

- Add Feature ID `DECODE-TILE-PARTITION-ALLOWED-BOUNDARY` for a crate-private
  decoder boundary over `partition_implied`, `partition_implied_at_boundary`,
  `rect_type_implied_by_bsize`, `is_partition_allowed`, and
  `init_allowed_partitions`.
- Add typed inputs for block position, MI frame bounds, tree type,
  subsampling, partition feature flags, BRU-independent region facts, chroma
  offset state, and the temporary caller-provided chroma-64x64 luma partition
  fact needed before recursive traversal owns `ChromaPartitionKnown` and
  `LumaPartitions`.
- Model the § 5.20.7.26 `get_plane_residual_size()` table locally enough for
  partition-allowance chroma checks, preserving `BLOCK_INVALID` as an invalid
  result instead of a valid block-size index.
- Reuse the existing `PartitionType`, `AllowedPartitions`,
  `RectPartitionType`, and `partition_size` checked block-size helpers.
- Add focused positive, edge, and negative tests for frame/tile boundaries,
  rectangular implication, mixed-region 4x4 rejection, aspect-ratio gating,
  chroma-offset gating, extended-partition feature flags, fallback to
  `PARTITION_NONE` when nothing is allowed, and checked arithmetic errors.
- Update the decoder support matrix, implementation matrix, roadmap/status
  docs, and OpenSpec archive artifacts to state the exact supported scope.
- Non-goals: no recursive `read_partition()`/`decode_partition()` traversal,
  no `decode_tile()`, no `MiSizes` mutation, no CDF mutation, no reconstruction,
  no output/hash/Y4M changes, no reference refresh, no public API behavior, no
  external decoder invocation, and no dependency graph change.

## Capabilities

### New Capabilities
- `tile-partition-allowed-boundary`: Crate-private AV2 § 5.20.3.2 partition
  implied and allowed-partition derivation boundary for future recursive tile
  partition traversal.

### Modified Capabilities

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/` gains a focused
  allowed-partition derivation module and tests; existing partition modules are
  reused rather than exposed publicly.
- Affected docs: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/DECODER-SUPPORT-STATUS.md`, and
  `docs/IMPLEMENTATION-MATRIX.toml`.
- APIs/dependencies: no public API changes, no new dependencies, no external
  decoder integration, and no crate dependency direction change.
- Validator/diagnostics impact: none; the boundary is crate-private and returns
  typed internal errors for invalid caller geometry instead of emitting
  user-facing diagnostics.
