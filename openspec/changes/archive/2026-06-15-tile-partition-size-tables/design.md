## Context

AV2 § 5.20.3.1 `decode_partition` and the § 5.20.3.2 helper functions use
§ 9.2 `Partition_Subsize[EXT_PARTITION_TYPES][BLOCK_SIZES]` and
`H_Partition_Midsize[BLOCK_SIZES]` to derive sub-block sizes after a partition
decision. The committed spec mirror contains both symbolic tables in
`docs/spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md`, and the
attachment mirror contains the same data in
`docs/spec/av2/1.0.0/attachments/all_tables.h`.

`cargo xtask gen-tables` currently emits numeric § 9 tables but explicitly skips
symbolic `BLOCK_*` tables, including `Partition_Subsize` and
`H_Partition_Midsize`, until the generator has a general enum-value map. The
decoder therefore needs a small crate-private boundary before later work can
derive `is_partition_allowed`, chroma offsets, and recursive partition traversal.

## Goals / Non-Goals

**Goals:**

- Add a crate-private decoder helper tracked by
  `DECODE-TILE-PARTITION-SIZE-TABLE-BOUNDARY`.
- Represent AV2 `BLOCK_*` table results with a typed `BlockSize` value that can
  distinguish `BLOCK_INVALID` from valid block sizes.
- Reuse the existing `PartitionType` enum from the partition decision boundary
  rather than introducing another partition ordering.
- Return typed errors for out-of-range partition and block-size indices before
  table indexing.
- Keep the helper self-contained, documented with spec citations, and covered by
  focused unit tests.

**Non-Goals:**

- No full symbolic table support in `cargo xtask gen-tables`.
- No public API exposure.
- No `partition_implied`, `init_allowed_partitions`,
  `is_partition_allowed`, recursive `read_partition()`/`decode_partition()`,
  `decode_tile()`, `exit_symbol()`, reconstruction, output, reference refresh,
  external decoder invocation, or scheduler/dependency change.

## Decisions

1. Add `crates/splot-decode/src/tile_payload/partition_size.rs`.

   Rationale: `partition.rs` and `cdf/context.rs` are each near the 1000-line
   source-file soft budget. A focused module keeps the new table surface isolated
   and gives later traversal code a single import point.

   Alternative considered: add the helpers to `partition.rs`. Rejected because it
   would grow an already large file with a separable table concern.

2. Generate the two symbolic partition-size tables from the committed
   attachment, then wrap them in decoder-local typed helpers.

   Rationale: the implementation matrix already says § 9 table contents should
   not be hand-transcribed. A narrow `BLOCK_*` resolver for `Partition_Subsize`
   and `H_Partition_Midsize` keeps table values generated from
   `all_tables.h` while avoiding a broad enum-symbol resolver for unrelated
   `TX_*`, `DCT_*`, and `reserved` tables.

   Alternative considered: hand-maintain the two arrays in `splot-decode`.
   Rejected because it would contradict the generated-table source-of-truth
   policy.

3. Use valid block-size discriminants matching the spec table order and a typed
   invalid sentinel.

   Rationale: existing generated § 9.2 arrays expose 29 `BLOCK_SIZES` entries in
   the normative order. Returning `Option<BlockSize>` or an enum with
   `Invalid` avoids converting `BLOCK_INVALID` into a usable index by mistake.

4. Reuse the existing partition ordering from `PartitionType`.

   Rationale: `PartitionType` already tracks AV2 § 6.19.3 values in the same
   order as `EXT_PARTITION_TYPES`. Making its index accessor visible within the
   tile-payload module is enough for table lookup without duplicating constants.

## Risks / Trade-offs

- [Risk] Hand-maintained table entries can drift from the spec mirror.
  → Mitigation: keep the table private, cite the mirror and attachment paths,
  test representative rows including invalid entries, and leave generator enum
  support as the long-term cleanup path.
- [Risk] Future callers might treat `BLOCK_INVALID` as a valid block size.
  → Mitigation: return a typed `PartitionSubsize` result with explicit
  `Valid(BlockSize)` and `Invalid` variants.
- [Risk] Out-of-range indices could panic if callers pass unchecked facts.
  → Mitigation: constructors and lookup helpers perform bounds checks and return
  typed errors before indexing arrays.
- [Risk] This can be overread as full partition traversal support.
  → Mitigation: matrix and roadmap notes state that traversal, allowed-partition
  derivation, and reconstruction remain out of scope.
