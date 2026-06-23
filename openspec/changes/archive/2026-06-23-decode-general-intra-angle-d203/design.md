# Design — DECODE-GENERAL-INTRA-ANGLE-D203

## § 7.13.2.8 zone-3 one-sided path

For `pAngle > 180` (step 3, `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-8`,
~lines 5845+):

```
dy       = Dr_Intra_Derivative[270 - pAngle]    (D203 -> Dr[67] = 24)
idx      = (j + 1 + mrlIndex) * dy
base     = (idx >> 6) + i
shift    = (idx >> 1) & 0x1F
maxBaseY = w + h - 1 + (mrlIndex << 1)
if (base < maxBaseY + enableIdif)   // enableIdif == 1 (luma) -> base <= maxBaseY
    if (enableIdif) s = Σ(t=0..3) Dr_Interp_Filter[shift][t] * LeftCol[base + t - 1]
                    pred = Clip1(Round2(s, 7))
    else            pred = Round2(LeftCol[base]*(32-shift) + LeftCol[base+1]*shift, 5)
else                pred = LeftCol[maxBaseY]
```

This is the exact symmetric mirror of the zone-1 step-1 (D45) path: zone-1 scales
`(i + 1)` by `dx` and offsets by `j`, reading `AboveRow`; zone-3 scales `(j + 1)`
by `dy` and offsets by `i`, reading `LeftCol`. `needBottom = pAngle > 180`, so the
projection reads DOWN-AND-LEFT: `base` ranges to `maxBaseY = w + h - 1` (= 127 for
64x64), i.e. the left column PLUS `w` below-left samples — the part of the
§ 7.13.2.1 reference column the MIDDLE angles never index.

D203's `dy = 24` makes `shift = ((j + 1) * 24 >> 1) & 0x1F = ((j + 1) * 12) & 0x1F`
NONZERO for most `j` (for 64x64 the distinct shifts are {0, 4, 8, 12, 16, 20, 24,
28}), so the IDIF 4-tap GENUINELY interpolates over the real reconstructed left
column — unlike D45 (`shift == 0`, degenerate sample copy).

## Recon kernel (splot-recon)

The existing zone-1 one-sided IDIF kernel
(`predict_intra_directional_angle_rect_one_sided_idif_into`) is GENERALISED to both
one-sided directions. `IntraDirectionalAngleIdifEdges` now carries the prepared
edge plus its direction (`above` for zone-1, `left` for zone-3); the validate /
reference / write paths dispatch on the angle's `DirectionalAngleBranch`
(`Above { dx }` for D45/D67, `Left { dy }` for D203). The per-sample reference is
`(scaled, offset) = (row, column)` for the above branch and `(column, row)` for the
left branch; `idx = (scaled + 1) * derivative`, `base = (idx >> 6) + offset`,
`shift = (idx >> 1) & 0x1F`. The shared `idif_tap` / `Dr_Interp_Filter` table and
the `-2 ..= w + h + 1` logical edge layout are reused unchanged. The left edge view
(`IntraDirectionalAngleIdifEdges::left`) carries `LeftCol[-2 ..= w + h + 1]`
(length `w + h + 4`, `slice[0]` = logical `-2`).

## Decode-side edge builder (splot-decode)

`reconstruct_general_intra_one_sided_left_neighbour_block_into` materialises the
full left column + below-left from the partially-built frame, per § 7.13.2.1's
`haveLeft == 1` branch:
- `LeftCol[i] = CurrFrame[plane][Min(leftLimit, y+i)][x-1]` for `i in 0..=maxBaseY`,
  with `leftLimit = Min(maxY, y + h + 4 * num4BelowLeft - 1)`.
- The corner `LeftCol[-1] = CurrFrame[plane][y][x-1]` (the `haveAbove == 0 &&
  haveLeft == 1` branch sets `LeftCol[-i] = AboveRow[-i] = CurrFrame[plane][y][x-1]`)
  and its extension `LeftCol[-2] = LeftCol[-1]`.
- The trailing extension `LeftCol[maxBaseY+1] = LeftCol[maxBaseY+2] =
  LeftCol[maxBaseY]`.

`num4BelowLeft` (in plane 4x4 units) is `full_sb_num4_below_left(r, n4h, sub_y)`
(§ 5.20.7.25 `count_bottom_left_avail` over the § 5.20.2.3 `BlockDecoded` state);
for a first-superblock-row, non-first-column full superblock no decoded superblock
sits below-left in raster order, so it is `0` and the below-left clamps to
`LeftCol[maxY]` (the last in-block left sample). The rightmost-clamp / non-zero
below-left positions are deferred.

Luma runs the zone-3 IDIF; chroma (D203-follow) runs the bilinear one-sided branch
(`enableIdif = plane == 0` is `0` for U/V) over the same prepared left edge view
(logical `0..w+h`), the spec-mandated chroma branch.

## Why the verified subset is this narrow

A directional luma block stores a directional `IntraJointMode`, which raises § 8.3.2
ctx to `1` for its neighbours; the § 5.20.5.3 directional-neighbour mode reorder is
unmodeled, so the D203 block's left and above neighbours MUST be non-directional
(DC). The fixture's gated position is the first superblock row (`haveAbove == 0`),
non-first column (`haveLeft == 1`): the left neighbour is a DC superblock carrying a
vertical-gradient residual (a non-flat real reconstructed right column), which makes
D203 selected over the flat alternatives. A row>0 D203 (which reads the above-left
corner from a decoded above superblock) and the first-column / top-left positions
(no real left column) are deferred until an oracle fixture pins them.

## Fixture

`syn-d203-intra-128x64-q80.ivf` (128x64, two superblock columns by one row):
LEFT 64x64 superblock DC_PRED carrying a vertical-gradient residual (non-flat right
column, 31..210); RIGHT 64x64 superblock (`frontier.r == 0`, `frontier.c == 16`,
`haveAbove == 0 && haveLeft == 1`) D203_PRED luma + `uv_mode == 0` directional-follow
D203 chroma. Decodes bit-exact to avmdec (`--rawvideo --i420`) AND dav2d
(`--demuxer ivf --muxer yuv`); raw md5 `2789636ec6bca9efcac829bbd7ca3712`, pinned
splot frame hash
`3b95907f8808cc9d0bdd2eb376c8726019f7a4490cf8ecfcccab883fb11f8a3f`.
