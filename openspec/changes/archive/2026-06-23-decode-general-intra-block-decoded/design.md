## Context

The general intra decoder reconstructs a partition tree in § 5.20.3.1 decode
order. DC blocks (§ 7.13.2.10) read only the immediate left column / above row,
whose availability is frame-position-based, so `syn-deep` decoded arbitrarily
deep DC splits with no § 5.20.2.3 `BlockDecoded` state. SMOOTH blocks
(§ 7.13.2.13) and directional blocks (§ 7.13.2.8) additionally read the
§ 7.13.2.1 above-right (`AboveRow[w]`) and below-left (`LeftCol[h]`) corner
sentinels, whose number of valid samples (`num4AboveRight` / `num4BelowLeft`)
§ 7.13.2.1 derives from § 5.20.7.25 `count_top_right_avail` /
`count_bottom_left_avail` over `BlockDecoded`.

Until now the general intra path approximated `num4AboveRight` with
`full_sb_num4_above_right`, which is only correct for a full-superblock block
(whose sub-block position is `(0, 0)` so its above-right is read directly from
the `clear_block_decoded_flags` above-row marking). A SPLIT child has a non-zero
superblock-relative position and its above-right may be an already-decoded
intra-superblock sibling, which the approximation cannot count. So a
sub-partitioned SMOOTH block was rejected.

## Goals / Non-Goals

- Goal: a faithful § 5.20.2.3 `BlockDecoded` grid that lets a SMOOTH_H luma
  SPLIT sub-block read the correct § 7.13.2.1 above-right sentinel.
- Goal: keep the verified subset narrow — only the SMOOTH_H luma SPLIT sub-block
  proven bit-exact is admitted; everything else (SMOOTH_V below-left, SMOOTH
  chroma sub-blocks, directional sub-blocks) still rejects.
- Non-goal: a runtime change for DC blocks (they already work) or any new public
  API, dependency, encoder, or validator behavior.

## Decisions

### `BlockDecoded` is modelled superblock-relative

Per § 5.20.2.3, `BlockDecoded[plane][y][x]` is indexed from the current
superblock origin (`subBlockMiRow = row & sbMask`, `subBlockMiCol = col & sbMask`),
with `y` in `[-1, sbSize4 >> subY]` and `x` in `[-1, (2 * sbSize4) >> subX]` and
re-initialized at each superblock by `clear_block_decoded_flags`. The grid is
therefore a small per-superblock structure (cleared each superblock), not a
full-tile grid; the `-1` edges are stored at offset `+1`. `clear_superblock`
mirrors the spec double loop exactly (including the corner `[-1][-1] = 1`).

### The walk owns the grid; the leaf reads it

`decode_general_intra_partition_tree` creates the grid from the frame facts
(`NumPlanes`, `SubsamplingX/Y`, `sbSize4`, `MiColEnd`, `MiRowEnd`), clears it per
superblock, passes it read-only to the `on_leaf` callback (so the leaf can query
`count_top_right_avail`), and marks each decoded block's plane 4x4 units via
`set_block` after the leaf returns. Under the verified subset's TX_MODE_LARGEST
each block is a single full-block transform, so the plane 4x4 extent is the
block's plane 4x4 width / height.

### Verified-subset discipline

The luma above-right derivation switches to `count_top_right_avail` over the real
grid for the existing neighbour-having non-DC luma path; for a full-superblock
block this is bit-identical to `full_sb_num4_above_right` (both implement the
same spec function and the grid is now correctly maintained), so the verified
full-SB SMOOTH paths are unchanged. The new admission arm admits only a
SMOOTH_H luma SPLIT sub-block of size >= 32x32 (TX_SET_DCTONLY); SMOOTH_V luma
sub-blocks and SMOOTH chroma sub-blocks remain rejected with structured
diagnostics, pinned by the negative `syn-svsplit` fixture.

## Risks / Trade-offs

- A SPLIT child's above-right read could be wrong if the grid's superblock-clear
  or per-block-set were off-by-one. Mitigated by faithful § 5.20.2.3 modelling,
  unit tests over the grid (including the bottom-left-reads-top-right case), and
  the bit-exact oracle gate, which fails on any mismatch (the reconstructed
  right column 211 vs the clamp's ~51 is a ~160-level difference, so a wrong
  sentinel would not produce the pinned hash).
