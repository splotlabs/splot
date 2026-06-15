## Context

`splot-decode` already has three crate-private partition prerequisites:

- `tile-partition-symbol-read-boundary` reads individual partition `S()`
  symbols through bounded CDF selectors.
- `tile-partition-decision-boundary` consumes caller-provided implied and
  allowed-partition facts to follow the AV2 § 5.20.3.2 branch order.
- `tile-partition-size-tables` maps § 9.2 `Partition_Subsize` and
  `H_Partition_Midsize` entries into checked block-size values or
  `BLOCK_INVALID`.

The remaining gap before recursive traversal can use the decision boundary is
deriving the implied partition and allowed set from AV2 geometry and feature
flags. The spec logic lives in § 5.20.3.2 and references § 5.20.7.26
`get_plane_residual_size()`.

## Goals / Non-Goals

**Goals:**

- Add a new crate-private `tile_payload::partition_allowed` module.
- Provide typed derivation for:
  - `rect_type_implied_by_bsize(bSize)`;
  - `partition_implied_at_boundary(r, c, bSize)`;
  - `partition_implied(r, c, bSize)`;
  - `is_partition_allowed(...)`;
  - `init_allowed_partitions(...)`.
- Keep every table/index/coordinate access bounded and panic-free.
- Preserve `BLOCK_INVALID` as an invalid residual/subsize result.
- Reuse existing `PartitionType`, `AllowedPartitions`, `RectPartitionType`, and
  `partition_size::BlockSize` instead of adding public APIs.

**Non-Goals:**

- No recursive `read_partition()` / `decode_partition()` / `decode_tile()`.
- No symbol reads, CDF mutation, `exit_symbol()`, or Saved CDF updates.
- No `MiSizes`, `ChromaPartitionKnown`, or `LumaPartitions` storage mutation.
- No reconstruction, hash/Y4M/raw output, reference refresh, or public runtime
  decode claim.
- No AVM/dav2d invocation or checked-in integration.

## Decisions

1. **New sibling module instead of growing `partition.rs`.**

   `partition.rs` is already close to the 1000-line advisory budget and owns
   the decision consumer. A sibling `partition_allowed.rs` keeps the derivation
   boundary testable without making the decision module a general traversal
   module.

2. **Explicit input facts instead of implicit traversal state.**

   The boundary will accept a struct containing MI frame bounds, block origin,
   tree type, subsampling, partition feature flags, `FrameIsIntra`,
   `RegionType == MIXED_REGION`, `MaxPbAspectRatio`, `hasChroma`,
   `chromaOffset`, `NumPlanes`, and an optional known luma partition for the
   chroma `BLOCK_64X64` branch. This mirrors the spec variables while keeping
   traversal-owned arrays out of scope.

3. **Generated-geometry § 5.20.7.26 residual-size derivation.**

   `Subsampled_Size` is not part of the generated § 9 table attachment. The
   module derives the residual block dimensions from generated block-size
   geometry and converts the result through `BlockSize` /
   `PartitionSubsize`-style invalid handling. A crate-private exhaustive test
   locks the derivation against the literal AV2 § 5.20.7.26 table. This avoids
   production hand-transcription drift while keeping the dependency local and
   spec-cited.

4. **Typed internal errors for invalid caller facts.**

   Invalid block-size indices are already rejected by `BlockSize`. This module
   additionally returns typed errors for arithmetic overflow or impossible table
   values. Normal AV2 disallow decisions return `false` or an allowed-set result;
   they do not become user-facing diagnostics in this PR.

## Risks / Trade-offs

- [Risk] The chroma `BLOCK_64X64` implied branch depends on traversal-owned
  `ChromaPartitionKnown` and `LumaPartitions`.
  → Mitigation: take a single optional caller-provided luma partition fact and
  document that array ownership remains future traversal work.
- [Risk] Deriving § 5.20.7.26 `Subsampled_Size` geometrically can drift.
  → Mitigation: add an exhaustive table-lock test for every block-size,
  `SubsamplingX`, and `SubsamplingY` combination, and cite the exact mirror
  section.
- [Risk] Geometry arithmetic can overflow with hostile caller-provided values.
  → Mitigation: use checked arithmetic for all `r/c + offset` and scaled
  4-way offsets, and test `usize::MAX` inputs.
- [Risk] Full allowed-partition derivation may look like full traversal support.
  → Mitigation: matrix/docs state this remains a crate-private boundary with no
  syntax reads, recursion, `MiSizes` mutation, reconstruction, or output.
