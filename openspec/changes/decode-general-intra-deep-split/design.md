## Context

The `syn-quad` fixture proves a single SPLIT (64x64 -> four 32x32 DC). Real AV2
intra frames nest partitions deeper. The partition recursion
(`child_calls` PARTITION_SPLIT in `partition_traversal.rs`) and the per-leaf DC
reconstruction (`reconstruct_general_intra_block_into` over the persistent
`CurrentFrameWorkspace`) already exist and are size-generic. The open question is
whether a deeper square SPLIT (a sub-32x32 16x16 leaf predicting from a sibling
16x16 inside the parent 32x32) is decoded bit-exactly today, and whether it
needs the § 5.20.2.3 `BlockDecoded` flag state.

## Goals / Non-Goals

**Goals:**
- Prove a two-level square SPLIT (sub-32x32 -> 16x16 DC leaves) decodes
  bit-exactly to the avmdec/dav2d oracle.
- Show that each 16x16 leaf DC-predicts from its reconstructed sibling neighbour
  inside the parent 32x32 sub-block.

**Non-Goals:**
- No § 5.20.2.3 `BlockDecoded` flag state (not needed for DC; deferred for the
  non-DC / SMOOTH sub-superblock above-right / below-left sentinels).
- No rectangular-leaf or non-DC sub-32x32 partitions, non-64x64 frames, inter
  prediction, or in-loop filters.
- No live in-CI AVM/dav2d dependency.

## Decisions

1. The deeper square DC split needs no new production code.

   Rationale: the § 7.13.2.4 DC predictor reads only the immediate left column
   (`x - 1`) and above row (`y - 1`). The workspace derives left/above
   availability from frame position (`rect.x() == 0` / `rect.y() == 0`), and the
   partition walk reconstructs leaves in § 5.20.3.1 decode (DFS) order, so a
   16x16 leaf always reads the already-reconstructed sibling 16x16 neighbour
   inside its parent 32x32. This is exactly the § 5.20.2.3 left/above
   availability the DC predictor requires; the § 5.20.2.3 `BlockDecoded` flag
   array is only consulted by `count_top_right_avail` for the § 7.13.2.1
   above-right / below-left sentinels, which the DC predictor never reads. The
   deeper split is therefore bit-identical to the verified shared code at a
   smaller transform log2 and one recursion level deeper.

2. Admit the deeper split only under the verified-subset discipline.

   Rationale: the existing leaf gate already admits any square DC block of >= 8x8
   luma with chroma present, which covers the 16x16 leaf provably (bit-identical
   to the verified 32x32 path). A non-DC or rectangular sub-32x32 leaf is still
   rejected with `decode/unsupported-feature` before it can desynchronise the
   decoder, so the deeper split admits ONLY the proven (square, DC) combination.

## Risks / Trade-offs

- [Risk] A latent neighbour-availability bug could be masked by flat content.
  -> Mitigation: each 16x16 leaf carries a distinct DC value (240/20/20/240) so a
  wrong neighbour read would change the decoded pixels; the § 8.2.4
  `exit_symbol()` check after the whole tile is a strong bit-exactness guard; the
  raw output is byte-compared against BOTH avmdec and dav2d and the frame hash is
  pinned.
- [Risk] Encoder RDO may emit rectangular or non-DC leaves for fine detail.
  -> Mitigation: the committed fixture's content + base_q_idx were chosen so the
  encoder emits exactly four square 16x16 DC leaves (verified via temporary
  partition/mode instrumentation, since removed); a rectangular or non-DC deeper
  split still rejects.
