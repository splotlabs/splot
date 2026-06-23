## Why

The general intra decode reconstructs all three § 7.13.2.8 MIDDLE directional
angles (`90 < pAngle < 180`: `D135_PRED`, `D157_PRED`, `D113_PRED`), the two
cardinal copies (`V_PRED` pAngle 90, `H_PRED` pAngle 180), and the first
§ 7.13.2.8 ZONE-1 ONE-SIDED angle `D45_PRED` (pAngle 45, `needRight`, reads the
ABOVE-RIGHT). The symmetric § 7.13.2.8 ZONE-3 ONE-SIDED angles (`pAngle > 180`,
`needBottom`) are still entirely rejected. `D203_PRED` (pAngle 203, canonical
§ 9.2 mode 7) is the diagonal down-left mode whose
`dy = Dr_Intra_Derivative[270 - 203] = Dr_Intra_Derivative[67] = 24` projects
DOWN-AND-LEFT into the BELOW-LEFT: `idx = (j + 1) * dy`, `base = (idx >> 6) + i`,
up to `maxBaseY = w + h - 1`. Unlike the MIDDLE angles (which read
`AboveRow[0..w)` / `LeftCol[0..h)`), the zone-3 projection reads `w` real
reconstructed LEFT-column samples (and the clamped below-left) — the symmetric
mirror of the D45 above-right zone.

D203 is the realistically encoder-selectable zone-3 angle on a first-superblock
row: an avmenc minimal-tool encode picks `D203_PRED` for a clean down-left block
whose left neighbour is a non-flat DC superblock (which keeps § 8.3.2 ctx == 0).
Unlike D45 (`dx = 64`, every `shift == 0`, the IDIF reduces to a copy), D203's
`dy = 24` lands most projections on a NONZERO shift, so the § 7.13.2.8 luma IDIF
4-tap genuinely interpolates over the real reconstructed left column — exercising
the IDIF kernel in its zone-3 direction.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-ANGLE-D203`.
- Generalise the one-sided luma IDIF kernel in `splot-recon`
  (`predict_intra_directional_angle_rect_one_sided_idif_into` now dispatches on the
  angle branch) and add `IntraDirectionalAngleIdifEdges::left` carrying the
  prepared left edge `LeftCol[-2 ..= w + h + 1]` (length `w + h + 4`); it reuses
  the existing § 7.13.2.8 / § 9.2 `Dr_Interp_Filter[32][4]` table unchanged
  (`RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`).
- Extend `IntraYMode::supported_directional` to map mode value 7 to
  `SupportedDirectionalLumaMode::D203`, and add `SupportedChromaMode::D203Follow`
  (resolved when `uv_mode == 0` over the D203 luma makes § 5.20.5.3 return
  `UVMode == D203_PRED`, `AngleDeltaUV == AngleDeltaY == 0`).
- Add the decode-side
  `reconstruct_general_intra_one_sided_left_neighbour_block_into`, which builds the
  § 7.13.2.1 left edge `LeftCol[i] = CurrFrame[plane][Min(leftLimit, y+i)][x-1]`
  (`leftLimit = Min(maxY, y + h + 4 * num4BelowLeft - 1)`, `num4BelowLeft` from
  § 5.20.7.25 `count_bottom_left_avail` over the § 5.20.2.3 `BlockDecoded` state,
  `0` in raster order) + the corner `LeftCol[-1] = CurrFrame[plane][y][x-1]` (the
  `haveAbove == 0 && haveLeft == 1` branch) + the § 7.13.2.8 edge extensions, then
  runs the zone-3 luma IDIF (luma) or the bilinear one-sided branch (chroma).
- Admit ONLY the verified subset: a first-superblock-row, NON-first-column full
  64x64 superblock (`frontier.r == 0 && frontier.c != 0`, `n4w == 16`,
  `haveAbove == 0 && haveLeft == 1`) D203 luma block and its `uv_mode == 0`
  directional-follow D203 chroma. Keep every other position (top-left, first-column,
  row>0, sub-partitioned, non-64x64) rejected, and keep the last unsupported
  one-sided angle D67, non-zero angle deltas, and the directional-neighbour
  (`ctx != 0`) escape reorder rejected.
- Add the `syn-d203-intra-128x64-q80.ivf` fixture, its conformance manifest
  entry, the decoder support row, the decode matrix row, and the reciprocal
  LOCAL-REFERENCE-EVIDENCE entry.

## Impact

- Affected specs: `decode-general-intra-angle-d203`, `decoder-support`.
- Affected code: `crates/splot-recon/src/intra_directional_angle.rs`,
  `crates/splot-decode/src/tile_payload/cdf/block_context.rs`,
  `crates/splot-decode/src/runtime_minimal/general_intra.rs`,
  `crates/splot-decode/src/runtime_minimal_recon.rs`.
- No dependency-graph change, no new dependency, no public CLI surface change.
