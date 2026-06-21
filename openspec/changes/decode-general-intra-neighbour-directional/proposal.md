## Why

The general intra decode reconstructs a § 7.13.2.8 `D135_PRED` (pAngle 135) luma
block and its `uv_mode == 0` directional-follow D135 chroma only at the
no-neighbour top-left 64x64 superblock, over the § 7.13.2.1 flat fallback edges
(`DECODE-GENERAL-INTRA-ANGLE`, `DECODE-GENERAL-INTRA-DIRECTIONAL-FOLLOW-CHROMA`).
A D135 block at any other superblock position — reading a REAL reconstructed
neighbour edge — was rejected with `general_intra_multiblock_directional_luma`
(luma) and `general_intra_directional_chroma_neighbour` (chroma), on the
assumption that over a non-flat edge the § 7.13.2.8 luma IDIF 4-tap differs from
the bilinear branch.

That assumption does NOT hold for pAngle 135. Its derivatives are
`dx = dy = Dr_Intra_Derivative[45] = 64`, so every § 7.13.2.8 projection has
`idx` a multiple of 64 and `shift = (idx >> 1) & 0x1F == 0`. At `shift == 0` the
luma IDIF 4-tap (`enableIdif == 1`) collapses to `Dr_Interp_Filter[0] =
{0, 128, 0, 0}`, i.e. `Clip1(Round2(128 * Edge[base], 7)) == Edge[base]`, which is
bit-identical to the chroma bilinear branch (`enableIdif == 0`:
`Round2(Edge[base] * 32 + Edge[base + 1] * 0, 5) == Edge[base]`) — a pure sample
copy `Edge[base]` EVEN OVER A NON-FLAT reconstructed edge. So no new IDIF 4-tap
kernel is needed for D135; the existing shared bilinear middle-angle predictor is
exact for D135 in both planes, and the only new work is reading the REAL
§ 7.13.2.1 edges instead of the flat fallback. Lifting the reject (verified
bit-exact against avmdec AND dav2d) is the first general-intra neighbour-having
directional decode.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-NEIGHBOUR-DIRECTIONAL`.
- Admit a first-superblock-row (`frontier.r == 0`, `haveAbove == 0`), non-top-left,
  full 64x64 superblock (`n4w == 16`) D135 luma block (`ctx == 0`, decoded via the
  § 5.20.5.3 `y_mode_offset` escape over a non-directional neighbour) and its
  `uv_mode == 0` directional-follow D135 chroma, reading the REAL reconstructed
  LEFT column.
- Add the plane-general `reconstruct_general_intra_directional_neighbour_block_into`
  and `build_directional_middle_edges`, which build the logical
  `AboveRow[-1..w)` / `LeftCol[-1..h)` edges from the partially-built frame
  faithful to § 7.13.2.1 (`MrlIndex == 0`, `enable_intra_edge_filter == 0`, no
  DIP/upsample) and run the shared bilinear middle-angle predictor (bit-exact for
  D135 by the `shift == 0` argument).
- Route `D135Follow` chroma with `x > 0 || y > 0` through the same neighbour path
  in `reconstruct_general_intra_chroma_block_into`.
- Keep deferred (still rejected): a row>0 D135 block reading the real above row
  (`general_intra_multirow_directional_luma`), sub-superblock directional blocks
  (`general_intra_multiblock_directional_subblock`), a directional NEIGHBOUR
  (`ctx != 0`) D135 escape (`general_intra_directional_neighbour_reorder`), other
  directional angles and non-zero angle deltas (`shift != 0`, real IDIF differs
  from bilinear), non-64x64 frames, inter prediction, and in-loop filters.
- Add the project-owned `syn-rdir-intra-128x64-q80.ivf` fixture (LEFT SMOOTH_V_PRED
  luma + DC chroma; RIGHT D135_PRED luma + directional-follow D135 chroma reading
  the real reconstructed left column) and prove it decodes bit-exactly to the
  avmdec AND dav2d oracle.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-neighbour-directional`: Crate-private general intra
  neighbour-having directional (`D135_PRED`) luma plus directional-follow D135
  chroma decode over a real reconstructed § 7.13.2.1 edge.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra neighbour-having directional decode.

## Impact

- Affects `crates/splot-decode/src/runtime_minimal_recon.rs` (the new
  `reconstruct_general_intra_directional_neighbour_block_into` /
  `build_directional_middle_edges` and the `D135Follow` chroma routing) and
  `crates/splot-decode/src/runtime_minimal/general_intra.rs` (the luma + chroma
  admission gates and dispatch). No new public surface; the recon prediction
  helper is reused.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and
  generated status docs.
- No dependency graph, encoder, or validator changes. A row>0 D135 block, a
  directional NEIGHBOUR (`ctx != 0`) escape, sub-superblock directional blocks,
  other directional angles / non-zero deltas, non-64x64 frames, inter prediction,
  and in-loop filters remain out of scope and rejected.
