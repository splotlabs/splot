## ADDED Requirements

### Requirement: Sub-pel inter prediction convolution kernel

The repository SHALL provide a scheduler-free `splot-recon` transcription of the
AV2 § 7.13.3.18 block inter prediction (sub-pel motion compensation) process,
tracked by `RECON-SUBPEL-MC`. The `subpel_predict_block(reference, params) ->
Result<Vec<u16>>` function SHALL run the separable interpolation-filter
convolution for a single-reference (non-compound) inter block: a horizontal
filter pass building an `intermediateHeight * w` array with `Round2(s,
InterRound0)`, then a vertical pass producing the `h * w` output, with the filter
taps selected from a verbatim § 7.13.3.18 `Subpel_Filters[6][16][8]` table by the
§ 6 `interp` (with the § 7.13.3.18 small-block 4-tap substitution applied per pass
keyed on `w` then `h`) and the sub-pel phase `(p >> 6) & SUBPEL_MASK`. It SHALL
apply `InterRound0 = 3` and `InterRound1 = 11` (the non-compound § 7.13.3.16
rounding) and the final § 4.8 `Clip1` for the § 7.13.3 single-reference write. It
SHALL implement the § 7.11.3.x reference-border extension via the § 7.13.3.18
`Clip3` to `[firstX, lastX] x [firstY, lastY]` plus the reference view's own
dimension clamp. The § 7.13.3.17 `startX` / `startY` / `stepX` / `stepY` scaling,
the § 7.13.3.18 reference-clipping region, the § 6 `interp_filter`, the block
dimensions, and the reference samples SHALL be caller-resolved. The function SHALL
be total and panic-free — validating the reference buffer length and non-zero
dimensions, rejecting a zero / oversized (`> 128`) block and a negative step, and
keeping every reference and intermediate access in bounds — and SHALL NOT
implement the § 7.13.3.17 motion-vector scaling, the § 7.13.3 compound /
mask-blend / distance-weighted prediction, the § 7.13.3.19 block warp, intra block
copy, the § 5.20.7 `read_mv` / `interp_filter` symbol decode, the inter mode_info
/ partition syntax, or runtime decode wiring.

#### Scenario: Sub-pel convolution matches the spec

- **WHEN** `cargo test -p splot-recon subpel --locked` runs
- **THEN** the test suite covers the verbatim-table invariant (every
  `Subpel_Filters` row sums to 128 and all taps are even, plus distinctive-row
  spot checks), the hand-anchored worked examples (a full-pel position is a
  bit-exact reference copy, a flat reference reconstructs flat for any phase, a
  hand-computed `EIGHTTAP_SHARP` half-pel, the border-extension corner, 10-bit
  `Clip1`, the small-block 4-tap substitution, and the error cases), and a
  property test comparing `subpel_predict_block` against an independent in-test
  re-trace of the § 7.13.3.18 pseudocode
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation
