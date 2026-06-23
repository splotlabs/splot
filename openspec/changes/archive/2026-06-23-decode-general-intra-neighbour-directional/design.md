## Context

The general intra decode reconstructs a § 7.13.2.8 `D135_PRED` luma block and its
`uv_mode == 0` directional-follow D135 chroma only at the no-neighbour top-left
64x64 superblock, over the § 7.13.2.1 flat fallback edges
(`DECODE-GENERAL-INTRA-ANGLE`, `DECODE-GENERAL-INTRA-DIRECTIONAL-FOLLOW-CHROMA`).
A neighbour-having D135 block was rejected
(`general_intra_multiblock_directional_luma`,
`general_intra_directional_chroma_neighbour`) on the assumption that over a
non-flat reconstructed edge the § 7.13.2.8 luma IDIF 4-tap differs from the
bilinear branch.

## Decisions

- **D135 IDIF == bilinear by `shift == 0` (no new kernel).** pAngle 135 is a
  § 7.13.2.8 middle angle (`90 < pAngle < 180`) with
  `dx = dy = Dr_Intra_Derivative[45] = 64`. The branch-2 above projection
  `idx = (j << 6) - (i + 1) * dx = 64 * (j - i - 1)` and the left projection
  `idx = (i << 6) - (j + 1) * dy = 64 * (i - j - 1)` are both multiples of 64, so
  `shift = (idx >> 1) & 0x1F == 0` for every sample. At `shift == 0` the luma IDIF
  4-tap (`enableIdif == 1`) is `sum(Dr_Interp_Filter[0][t] * Edge[base + t - 1])`
  with `Dr_Interp_Filter[0] = {0, 128, 0, 0}`, i.e. `128 * Edge[base]`, and
  `Clip1(Round2(128 * Edge[base], 7)) == Edge[base]`. The bilinear branch
  (`enableIdif == 0`, chroma) is `Round2(Edge[base] * 32 + Edge[base + 1] * 0, 5)
  == Edge[base]`. Both are the SAME sample copy `Edge[base]`, EVEN over a non-flat
  edge (verified numerically and against the oracle). So the existing shared
  bilinear `predict_intra_middle_directional_angle_rect_into` is bit-exact for
  D135 in both planes — only the real § 7.13.2.1 edges are new. Other angles
  (`shift != 0`) genuinely differ and remain deferred.

- **Build the real § 7.13.2.1 edges via `build_directional_middle_edges`.** For the
  verified first-superblock-row block (`frontier.r == 0`, `haveLeft == 1`,
  `haveAbove == 0`): `LeftCol[i] = CurrFrame[plane][Min(leftLimit, y + i)][x - 1]`
  (the real reconstructed left column, bottom-left sentinel clamped because
  `num4BelowLeft == 0` in raster order), `AboveRow[i] = CurrFrame[plane][y][x - 1]`
  (the repeated first left sample), and the corner
  `AboveRow[-1] = LeftCol[-1] = CurrFrame[plane][y][x - 1]`. The helper covers all
  four `haveLeft`/`haveAbove` cases for totality, including the no-neighbour flat
  fallback (matching `predict_directional_noneighbour`). D135 never reads the
  above-right sentinel value (its projections stay within `AboveRow[0..w)` /
  `LeftCol[0..h)`), so no above-right resolver is needed; the corner is never read
  by D135 over both edges (the above branch reads `base >= 0`, the left branch
  `base >= 0`).

- **`ctx == 0` is guaranteed.** The D135 escape was decoded via the § 5.20.5.3
  `y_mode_offset` escape; a directional joint-mode neighbour would have hit the
  deferred § 5.20.5.3 directional-neighbour reorder reject
  (`general_intra_directional_neighbour_reorder`) earlier, so a neighbour-having
  D135 escape that reaches reconstruction has a non-directional (DC/SMOOTH) left
  neighbour supplying the real left column.

- **Chroma is the directional-follow of the luma D135.** `uv_mode == 0` over the
  directional luma resolves `UVMode == D135_PRED`, `AngleDeltaUV == AngleDeltaY ==
  0`; `reconstruct_general_intra_chroma_block_into` routes `D135Follow` with
  `x > 0 || y > 0` (the right chroma block is at `chroma_x == 32`) to the same
  neighbour-having recon, reading the real reconstructed left chroma column via the
  bilinear branch (identical sample copy).

- **Gate to the verified subset.** Admit only `frontier.r == 0`, non-top-left,
  `n4w == 16`. A row>0 D135 block (real above row) is bit-exact by the same
  argument but is not yet oracle-fixtured
  (`general_intra_multirow_directional_luma`); sub-superblock directional blocks
  (`general_intra_multiblock_directional_subblock`) need the per-block § 5.20.2.3
  `BlockDecoded` update and mode-dependent transform type.

## Risks / Trade-offs

- The `build_directional_middle_edges` `(true, true)` (row>0) branch is written for
  totality but is not exercised by the committed fixture (gated off); its corner
  approximation is documented as never-read by D135. The row>0 path is deferred
  until a fixture pins it, so no unverified output is produced.

## Migration

None. Additive recon path plus widened admission gates; all existing general intra
fixtures decode to their previously pinned hashes (verified).
