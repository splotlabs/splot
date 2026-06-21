# Design — DECODE-GENERAL-INTRA-ANGLE-D45

## § 7.13.2.8 zone-1 one-sided path

For `pAngle < 90` (step 1, `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-8`,
~lines 5787+):

```
dx       = Dr_Intra_Derivative[pAngle]          (D45 -> Dr[45] = 64)
idx      = (i + 1 + mrlIndex) * dx
base     = (idx >> 6) + j
shift    = (idx >> 1) & 0x1F
maxBaseX = w + h - 1 + (mrlIndex << 1)
if (base < maxBaseX + enableIdif)   // enableIdif == 1 (luma) -> base <= maxBaseX
    if (enableIdif) s = Σ(t=0..3) Dr_Interp_Filter[shift][t] * AboveRow[base + t - 1]
                    pred = Clip1(Round2(s, 7))
    else            pred = Round2(AboveRow[base]*(32-shift) + AboveRow[base+1]*shift, 5)
else                pred = AboveRow[maxBaseX]
```

`needRight = pAngle < 90`, so the projection reads UP-AND-RIGHT: `base` ranges to
`maxBaseX = w + h - 1` (= 127 for 64x64), i.e. the above row PLUS `h` above-right
samples — the part of the § 7.13.2.1 reference row the MIDDLE angles never index.

D45's `dx = 64` makes `idx = (i + 1) * 64`, so `shift = ((i + 1) * 64 >> 1) &
0x1F == ((i + 1) * 32) & 0x1F == 0` for every projection, and `base = (i + 1) +
j`. The IDIF 4-tap row at `shift == 0` is `{0, 128, 0, 0}`, so `pred =
Clip1(Round2(128 * AboveRow[base], 7)) == AboveRow[base]` — bit-identical to the
bilinear branch, but indexing far into the real above-right.

## Recon kernel (splot-recon)

`predict_intra_directional_angle_rect_one_sided_idif_into` mirrors the existing
MIDDLE IDIF kernel but for the ABOVE-reading zone-1 angle. Its
`IntraDirectionalAngleIdifEdges::above(above_idif)` carries the wider above edge
`AboveRow[-2 ..= w + h + 1]` (length `w + h + 4`, `slice[0]` = logical `-2`),
because the IDIF reads `AboveRow[base - 1 ..= base + 2]` with `base` up to
`maxBaseX`, and the § 7.13.2.8 edge extension fills `AboveRow[maxBase + 1] =
AboveRow[maxBase + 2] = AboveRow[maxBase]` and `AboveRow[minBase - 1] =
AboveRow[minBase]`. The shared `idif_tap` / `Dr_Interp_Filter` table are reused
unchanged. Only D45 (the ABOVE-reading zone-1 angle) is admitted; D67 is also
zone-1 but is not exposed (no encoder-selectable fixture); D203 (zone-3, reads the
left + below-left) is a separate later kernel.

## Decode-side edge builder (splot-decode)

`reconstruct_general_intra_one_sided_neighbour_block_into` materializes the full
above row + above-right from the partially-built frame, per § 7.13.2.1:
- `AboveRow[i] = CurrFrame[plane][y-1][Min(aboveLimit, x+i)]` for `i in 0..=maxBaseX`,
  with `aboveLimit = Min(maxX, x + w + 4 * num4AboveRight - 1)`.
- The corner `AboveRow[-1] = CurrFrame[plane][y-1][x-1]` and its extension
  `AboveRow[-2] = AboveRow[-1]`.
- The trailing extension `AboveRow[maxBaseX+1] = AboveRow[maxBaseX+2] =
  AboveRow[maxBaseX]`.

`num4AboveRight` (in plane 4x4 units) is `full_sb_num4_above_right(c, n4w,
mi_cols, sub_x)` (§ 5.20.7.25 `count_top_right_avail` over the § 5.20.2.3
`BlockDecoded` state); for a non-rightmost row>0 full superblock it is the full
above-right superblock width, so the above-right is the real reconstructed bottom
row of the diagonally-above-right superblock. The rightmost position
(`num4AboveRight == 0`) clamps (degenerate) and is rejected.

Luma runs the zone-1 IDIF; chroma (D45-follow) runs the bilinear one-sided branch
(`enableIdif = plane == 0` is `0` for U/V) over the same edge view (logical
`0..w+h`), bit-identical to the luma copy at `shift == 0`.

## Why the verified subset is this narrow

A directional luma block stores a directional `IntraJointMode`, which raises § 8.3.2
ctx to `1` for its neighbours; the § 5.20.5.3 directional-neighbour mode reorder is
unmodeled, so the D45 block's left and above neighbours MUST be non-directional
(DC). A flat above (DC) makes a `shift != 0` zone-1 angle (D67) degenerate, so the
encoder will not select it over a flat above — and a non-flat directional above
neighbour would cascade ctx != 0. D45 (`shift == 0`) IS selected over a flat
above-middle when the upper-right triangle matches the non-flat ABOVE-RIGHT (a DC
block carrying a gradient residual), which is exactly the committed fixture.

## Fixture

`syn-d45-intra-192x128-q80.ivf` (192x128, three superblock columns by two rows):
top-left / top-middle / bottom-left / bottom-right DC_PRED; top-right DC with a
horizontal-gradient residual (non-flat reconstruction); BOTTOM-MIDDLE (frontier.r
16, frontier.c 16, haveLeft && haveAbove, non-rightmost) D45_PRED luma + uv_mode 0
D45-follow chroma. Decodes bit-exact to avmdec (`--rawvideo --i420`) AND dav2d
(`--demuxer ivf --muxer yuv`); raw md5 `8fe6a134c01b0d20be4016348ccd3b99`, pinned
splot frame hash `d08056c0d1ed3f379e3072c7f1ebced04da0f6df994efd0b5f8d39b76c0b683f`.
