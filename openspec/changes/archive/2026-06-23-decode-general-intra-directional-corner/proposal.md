## Why

The general intra decode reconstructs the § 7.13.2.8 `D135_PRED` (pAngle 135) and
`D157_PRED` (pAngle 157) middle directional modes, but ONLY in the first superblock
row (`frontier.r == 0`, `haveAbove == 0`). A row>0 directional block reads the
§ 7.13.2.1 corner `AboveRow[-1] == LeftCol[-1]`, which § 7.13.2.8 D135 reads on its
main diagonal (`column == row`, `above_base == -1`, `shift == 0`, a sample copy).
At `haveAbove == 1` that corner is the REAL reconstructed diagonally-above-left
sample `CurrFrame[plane][y-1][x-1]`, which `intra_dc_edges_for_rect` does not
return. The prior `build_directional_middle_edges` `(true, true)` and `(false, true)`
arms therefore `return Err(UnsupportedDirectionalAboveEdge)` rather than fabricate a
wrong corner, and the admission gate rejected the whole row>0 directional path
(`general_intra_multirow_directional_luma` for luma /
`general_intra_directional_chroma_neighbour` for chroma). This blocked ALL row>0
directional intra (D113/D135/D157 at superblock row>0), the prerequisite for the
vertical-leaning angles.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-DIRECTIONAL-CORNER`.
- Build the real § 7.13.2.1 corner in `build_directional_middle_edges`: the
  `haveLeft && haveAbove` arm takes the real `CurrFrame[plane][y-1][x-1]` (read by
  the caller via `reconstructed_sample`, `aboveMrlIndex == 0` at the superblock
  boundary, `MrlIndex == 0`) plus the real above row and left column; the
  `!haveLeft && haveAbove` arm takes the real above row and derives
  `LeftCol[i] = AboveRow[-1] = CurrFrame[plane][y-1][x] = AboveRow[0]`. The
  `(true, false)` and `(false, false)` arms are unchanged.
- Route the row>0 directional reconstruction through the built corner + the
  existing § 7.13.2.8 IDIF luma 4-tap / bilinear chroma kernel + the § 5.20.7.27
  residual.
- Admit ONLY the verified subset: a row>0, NON-first-column full 64x64 superblock
  (`frontier.r != 0 && frontier.c != 0`, `haveLeft && haveAbove`) D135 luma block
  and its `uv_mode == 0` directional-follow D135 chroma. Keep the no-corner
  first-row / no-neighbour D135/D157 paths byte-identical, and keep the row>0
  FIRST-column (`!haveLeft && haveAbove`) position, any row>0 D157, sub-superblock
  directional blocks, non-zero angle deltas, and the directional-neighbour
  (`ctx != 0`) escape reorder rejected.
- Add the `syn-d135row-intra-128x128-q80.ivf` fixture, its conformance manifest
  entry, the decoder support row, the decode matrix row, and the reciprocal
  LOCAL-REFERENCE-EVIDENCE entry.

## Impact

- Affected specs: `decode-general-intra-directional-corner`, `decoder-support`.
- Affected code: `crates/splot-decode/src/runtime_minimal_recon.rs`,
  `crates/splot-decode/src/runtime_minimal/general_intra.rs`,
  `crates/splot-decode/src/tile_payload/general_intra_residual.rs`.
- No dependency-graph change, no new dependency, no public CLI surface change.
