## Why

The general intra decode reconstructs all three § 7.13.2.8 MIDDLE directional
angles (`90 < pAngle < 180`): `D135_PRED`, `D157_PRED`, and `D113_PRED`, plus the
two cardinal copies (`V_PRED` pAngle 90, `H_PRED` pAngle 180). The § 7.13.2.8
ZONE-1 ONE-SIDED angles (`pAngle < 90`, `needRight`) are still entirely rejected.
The first of these, `D45_PRED` (pAngle 45, canonical § 9.2 mode 3), is the
diagonal up-right mode whose `dx = Dr_Intra_Derivative[45] = 64` projects
UP-AND-RIGHT into the ABOVE-RIGHT: `pred[i][j] = AboveRow[base]`, `base = (i + 1)
+ j`, up to `maxBaseX = w + h - 1`. Unlike the MIDDLE angles (which read
`AboveRow[0..w)` / `LeftCol[0..h)`), the zone-1 projection's upper-right triangle
reads `h` real reconstructed ABOVE-RIGHT samples — a part of the § 7.13.2.1
reference row the middle angles never touch. splot-recon's one-sided predictor was
bilinear-only over a `w+h` above edge (no IDIF, no above-right materialization
beyond the caller's flat slice), so this brick adds the zone-1 luma IDIF kernel
and the decode-side above-right edge builder.

D45 is the realistically encoder-selectable zone-1 angle: an avmenc minimal-tool
encode picks `D45_PRED` for a clean up-right-diagonal block surrounded by DC
neighbours (which keep § 8.3.2 ctx == 0). D45's `shift` is always `0`
(`(i + 1) * 64 >> 1 & 0x1F == 0`), so the § 7.13.2.8 IDIF 4-tap reduces to the
sample copy `AboveRow[base]` (bit-identical to the bilinear branch), but it still
reads far into the REAL reconstructed above-right — the decisive new capability.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-ANGLE-D45`.
- Add the zone-1 luma IDIF kernel to `splot-recon`
  (`predict_intra_directional_angle_rect_one_sided_idif_into`,
  `IntraDirectionalAngleIdifEdges::above`) over the wider above edge
  `AboveRow[-2 ..= w + h + 1]` (length `w + h + 4`); it reuses the existing
  § 7.13.2.8 / § 9.2 `Dr_Interp_Filter[32][4]` table unchanged
  (`RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`).
- Extend `IntraYMode::supported_directional` to map mode value 3 to
  `SupportedDirectionalLumaMode::D45`, and add `SupportedChromaMode::D45Follow`
  (resolved when `uv_mode == 0` over the D45 luma makes § 5.20.5.3 return
  `UVMode == D45_PRED`, `AngleDeltaUV == AngleDeltaY == 0`).
- Add the decode-side `reconstruct_general_intra_one_sided_neighbour_block_into`,
  which builds the § 7.13.2.1 above edge `AboveRow[i] =
  CurrFrame[plane][y-1][Min(aboveLimit, x+i)]` (`aboveLimit = Min(maxX, x + w +
  4 * num4AboveRight - 1)`, `num4AboveRight` from § 5.20.7.25
  `count_top_right_avail` over the § 5.20.2.3 `BlockDecoded` state) + the real
  corner + the § 7.13.2.8 edge extensions, then runs the zone-1 luma IDIF (luma)
  or the bilinear one-sided branch (chroma).
- Admit ONLY the verified subset: a row>0, NON-first-column, NON-rightmost full
  64x64 superblock (`frontier.r != 0 && frontier.c != 0`, `n4w == 16`,
  `full_sb_num4_above_right > 0`, `haveLeft && haveAbove`) D45 luma block and its
  `uv_mode == 0` directional-follow D45 chroma. Keep every other position
  (top-left, first-row `haveAbove == 0`, first-column, RIGHTMOST, sub-partitioned,
  non-64x64) rejected, and keep the other one-sided angles D67/D203, non-zero
  angle deltas, and the directional-neighbour (`ctx != 0`) escape reorder rejected.
- Add the `syn-d45-intra-192x128-q80.ivf` fixture, its conformance manifest
  entry, the decoder support row, the decode matrix row, and the reciprocal
  LOCAL-REFERENCE-EVIDENCE entry.

## Impact

- Affected specs: `decode-general-intra-angle-d45`, `decoder-support`.
- Affected code: `crates/splot-recon/src/intra_directional_angle.rs`,
  `crates/splot-recon/src/lib.rs`,
  `crates/splot-decode/src/tile_payload/cdf/block_context.rs`,
  `crates/splot-decode/src/runtime_minimal/general_intra.rs`,
  `crates/splot-decode/src/runtime_minimal_recon.rs`.
- No dependency-graph change, no new dependency, no public CLI surface change.
