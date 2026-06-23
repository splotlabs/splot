## Context

§ 7.13.2.8 sets `enableIdif = (plane == 0)` (luma always IDIF). The middle-angle
(`90 < pAngle < 180`) IDIF branch is
`s = Σ(t=0..3) Dr_Interp_Filter[shift][t] * Edge[base + t - 1]; pred = Clip1(Round2(s, 7))`
with `shift = (idx >> 1) & 0x1F`, reading `LeftCol` (or `AboveRow`) at
`base - 1 ..= base + 2`. The chroma branch is the 2-tap bilinear
`Round2(Edge[base] * (32 - shift) + Edge[base + 1] * shift, 5)`.

`Dr_Interp_Filter[32][4]` is the § 7.13.2.8 / § 9.2 constant table. It is NOT in
the generated `all_tables.h` attachment (so `cargo xtask gen-tables` cannot produce
it), so — like the existing `Dr_Intra_Derivative` entries in
`intra_directional_angle.rs` — it is a hand-authored local const COPIED VERBATIM
from `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-8`. Row 0 is
`{0, 128, 0, 0}`, so `shift == 0` gives `Clip1(Round2(128 * Edge[base], 7)) ==
Edge[base]` — a sample copy. This makes D135 (every projection `shift == 0`)
byte-identical under either branch.

## Decisions

- **IDIF as a separate primitive, not an in-place edit of the bilinear path.** The
  existing `predict_intra_middle_directional_angle_rect_into` (bilinear) is kept
  untouched (D135 chroma and existing tests stay byte-identical); a new
  `predict_intra_middle_directional_angle_rect_idif_into` takes wider IDIF edges
  (`Edge[-2..=side+1]`, length `side + 4`) because the 4-tap reads
  `Edge[base - 1 ..= base + 2]`. The spec edge extension (`Edge[minBase - 1] =
  Edge[minBase]`; for the middle branch `Edge[side] = Edge[side+1] =
  Edge[side-1]`) supplies the `-2` and `side`/`side+1` logical samples.

- **Signed Round2 + Clip1.** The 4-tap has negative taps, so the sum is signed.
  `Round2(x, 7) = ⌊(x + 64) / 128⌋` (floor, via arithmetic shift on the signed
  value; AVM's `ROUND_POWER_OF_TWO` agrees), then `Clip1` clamps a negative result
  to 0. Implemented as a dedicated `idif_round2_clip` helper with checked
  arithmetic.

- **D157 at `haveLeft && !haveAbove` (frontier.r == 0, frontier.c != 0).** At this
  position 3344 / 4096 samples take the left branch (the real reconstructed left
  column) and 2940 of those have `shift != 0` — genuinely exercising the 4-tap. The
  24 above-branch corner reads (`above_base == -1`) read the § 7.13.2.1 corner,
  which at `haveLeft && !haveAbove` is the repeated first-left sample
  (`CurrFrame[plane][y][x-1]`), so the deferred real-above corner is NOT a blocker.
  The top-left, first-column, and row>0 D157 positions read the no-neighbour /
  real-above corner that no fixture pins, so they are rejected
  (`general_intra_d157_unverified_position`).

- **D157-follow chroma via bilinear.** `enableIdif == 0` for chroma, so the chroma
  D157 follow uses the existing bilinear branch over the real (flat) reconstructed
  left chroma column; for the chroma 32x32 block the bilinear `base + 1` stays
  within `LeftCol[-1..side-1]`. Over the flat chroma edge the projection is
  bit-exact.

- **Verified subset.** Only D157 (and the unchanged D135) are admitted; D113 (the
  other middle angle) and the one-sided angles D45/D67/D203 stay rejected because
  no oracle fixture exercises them. The one-sided luma path keeps its
  `WorkspaceDirectionalAngleIntraPredictionLumaIdifUnsupported` reject.

## Oracle proof

The committed `syn-d157-intra-128x64-q80.ivf` decodes bit-exactly to avmdec
(authoritative) AND dav2d AND splot (raw md5 c8698fdb7628843971bc9e37a82391ae,
sha256 / pinned splot frame hash
bf93ca6b8f55e1fb7db2584f3e3821ad67f21018b774c6e326634362ee5ef046). Building
origin/main and decoding the same fixture confirms the OLD code rejected it.
