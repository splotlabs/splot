## Context

`RECON-SUBPEL-MC` is the § 7.13.3.18 block inter prediction process — the
separable interpolation-filter convolution that turns a fractional motion vector
and a reference plane into a sub-pel motion-compensated prediction block. The
zero-MV inter frontier (`DECODE-INTER-MOTION-COMPENSATION`) handles only the
full-pel co-located copy; this kernel is the general sub-pel math, mirroring how
the § 7.13.2.8 IDIF kernel landed ahead of its decode wiring.

## Goals / Non-Goals

**Goals:** a total, panic-free transcription of the § 7.13.3.18 two-pass
convolution and the verbatim § 9 `Subpel_Filters` table over caller-resolved
scaling, clipping region, filter, dimensions, and reference samples, producing the
final single-reference (non-compound) `Clip1` output.

**Non-Goals:** the § 7.13.3.17 motion-vector scaling (`startX`/`startY`/`stepX`/
`stepY` derivation), the § 7.13.3 compound / mask-blend / distance-weighted
prediction, the § 7.13.3.19 block warp, intra block copy, the reference-area
clipping selection, the § 5.20.7 `read_mv` / `interp_filter` symbol decode, the
inter mode_info / partition syntax, and any runtime wiring.

## Decisions

- **Produce the final `Clip1` output, not raw `Preds`.** For the single-reference
  (non-compound) case the § 7.13.3 write is
  `CurrFrame[plane][y + i][x + j] = Clip1(Preds[0][i][j])`, so the kernel returns
  the clipped reconstructed prediction samples directly (a `Vec<u16>`), matching
  the IDIF kernel's `Clip1(Round2(s, 7))` contract. Compound prediction (which
  keeps the unclipped 16-bit `Preds` for blending) is out of scope.
- **Caller-resolved scaling and clipping.** The § 7.13.3.17 motion-vector scaling
  produces `startX`/`startY` (the `x`/`y` § 7.13.3.18 inputs) and `stepX`/`stepY`
  in 1/1024-sample units; that derivation needs frame/reference dimensions, so the
  caller passes the four scalars. The § 7.13.3.18 reference-clipping region
  (`firstX`/`firstY`/`lastX`/`lastY`) is also a caller input, matching the spec.
- **Reference-border extension via Clip3 + view clamp.** The spec reads
  `ref[Clip3(firstY, lastY, ...)][Clip3(firstX, lastX, ...)]`; the
  `ReferencePlaneView` additionally clamps to its own `width`/`height` so the read
  is total even if a caller passes a clipping region wider than the actual plane
  (defense in depth). This avoids copying a padded reference plane.
- **Small-block 4-tap substitution per pass.** § 7.13.3.18 substitutes the 4-tap
  filter (index 4 for `EIGHTTAP`/`EIGHTTAP_SHARP`, index 5 for `EIGHTTAP_SMOOTH`)
  when the pass dimension is `<= 4`: keyed on `w` for the horizontal pass and `h`
  for the vertical pass, exactly as the spec's two `interpFilter` re-derivations.
- **`InterRound1 = 11` (non-compound).** § 7.13.3.16 sets `InterRound1` to
  `isCompound ? 7 : 11`; this kernel is the non-compound path, so it uses 11.
  `InterRound0` is always 3.
- **Verbatim table with an invariant guard.** `SUBPEL_FILTERS` is transcribed
  byte-for-byte from § 7.13.3.18; a test asserts the spec invariant (every row
  sums to 128, all taps even) plus distinctive-row spot checks so a transcription
  typo cannot pass.

## Risks / Trade-offs

- The table is large (768 coefficients) and the two-pass indexing is dense, so
  correctness rests on a faithful transcription and the spec's exact rounding /
  clipping / phase math. Mitigated by the verbatim-table invariant test, a
  2000-case property test against an independent in-test re-trace of the
  § 7.13.3.18 pseudocode, and hand-anchored worked examples (the full-pel copy,
  the flat-reference round-trip, a hand-computed half-pel, the border clip). It is
  loaded ahead of its runtime caller, matching the established IDIF / deblock
  pattern.
