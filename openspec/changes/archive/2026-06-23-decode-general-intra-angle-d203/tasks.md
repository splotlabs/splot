## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-ANGLE-D203` to the implementation matrix.
- [x] 1.2 Add the `general-intra-angle-d203` decoder support row.
- [x] 1.3 Add the `syn-d203-intra-128x64-q80.ivf` fixture, its conformance manifest entry, and the reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Generalise the § 7.13.2.8 one-sided luma IDIF kernel in `splot-recon` (`predict_intra_directional_angle_rect_one_sided_idif_into` dispatching on the angle branch) and add `IntraDirectionalAngleIdifEdges::left` over the wider left edge `LeftCol[-2 ..= w + h + 1]`, reusing the existing `Dr_Interp_Filter[32][4]` table unchanged (`RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`).
- [x] 2.2 Map mode value 7 to `SupportedDirectionalLumaMode::D203` (`IntraYMode::supported_directional`); add `SupportedChromaMode::D203Follow` and its § 5.20.5.3 `uv_mode == 0` directional-follow resolution.
- [x] 2.3 Add `reconstruct_general_intra_one_sided_left_neighbour_block_into`: build the § 7.13.2.1 left column + BELOW-LEFT (`CurrFrame[plane][Min(leftLimit, y+i)][x-1]`, `leftLimit` bounded by § 5.20.7.25 `num4BelowLeft`) + the `haveAbove == 0` corner + the § 7.13.2.8 edge extensions, then run the zone-3 luma IDIF (luma) / bilinear one-sided branch (chroma) + the § 5.20.7.27 residual.
- [x] 2.4 Admit ONLY the first-superblock-row, non-first-column full-superblock D203 luma block (`frontier.r == 0 && frontier.c != 0 && n4w == 16 && haveAbove == 0 && haveLeft == 1`) and its `uv_mode == 0` directional-follow D203 chroma; keep the top-left / first-column / row>0 / sub-partitioned / non-64x64 D203 positions, the last one-sided angle D67, non-zero angle deltas, and the directional-neighbour (`ctx != 0`) escape reorder rejected with structured `decode/unsupported-feature` diagnostics.

## 3. Documentation And Verification

- [x] 3.1 Add the D203 zone-3 decode-to-oracle test, the recon zone-3 IDIF kernel unit tests, the `supported_directional` admits-D203 unit test, and the D203-follow chroma resolution unit test; regenerate the feature/status/support/coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, conformance, and the Rust acceptance gate; confirm the fixture decodes bit-exact vs avmdec AND dav2d and that every existing general-intra fixture (V/H/D45/D113/D135/D157, cardinal, SMOOTH, DC) stays byte-identical and the last unsupported one-sided angle (D67) rejects.
