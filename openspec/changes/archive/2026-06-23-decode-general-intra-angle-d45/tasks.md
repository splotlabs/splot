## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-ANGLE-D45` to the implementation matrix.
- [x] 1.2 Add the `general-intra-angle-d45` decoder support row.
- [x] 1.3 Add the `syn-d45-intra-192x128-q80.ivf` fixture, its conformance manifest entry, and the reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Add the § 7.13.2.8 zone-1 luma IDIF kernel to `splot-recon` (`predict_intra_directional_angle_rect_one_sided_idif_into`, `IntraDirectionalAngleIdifEdges::above`) over the wider above edge `AboveRow[-2 ..= w + h + 1]`, reusing the existing `Dr_Interp_Filter[32][4]` table unchanged (`RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`).
- [x] 2.2 Map mode value 3 to `SupportedDirectionalLumaMode::D45` (`IntraYMode::supported_directional`); add `SupportedChromaMode::D45Follow` and its § 5.20.5.3 `uv_mode == 0` directional-follow resolution.
- [x] 2.3 Add `reconstruct_general_intra_one_sided_neighbour_block_into`: build the § 7.13.2.1 above row + ABOVE-RIGHT (`CurrFrame[plane][y-1][Min(aboveLimit, x+i)]`, `aboveLimit` bounded by § 5.20.7.25 `num4AboveRight`) + the real corner + the § 7.13.2.8 edge extensions, then run the zone-1 luma IDIF (luma) / bilinear one-sided branch (chroma) + the § 5.20.7.27 residual.
- [x] 2.4 Admit ONLY the row>0, non-first-column, NON-rightmost full-superblock D45 luma block (`frontier.r != 0 && frontier.c != 0 && n4w == 16 && full_sb_num4_above_right > 0`) and its `uv_mode == 0` directional-follow D45 chroma; keep the top-left / first-row / first-column / RIGHTMOST / sub-partitioned / non-64x64 D45 positions, the other one-sided angles D67/D203, non-zero angle deltas, and the directional-neighbour (`ctx != 0`) escape reorder rejected with structured `decode/unsupported-feature` diagnostics.

## 3. Documentation And Verification

- [x] 3.1 Add the D45 zone-1 decode-to-oracle test, the recon zone-1 IDIF kernel unit tests, and the `supported_directional` admits-D45 unit test; regenerate the feature/status/support/coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, conformance, and the Rust acceptance gate; confirm the fixture decodes bit-exact vs avmdec AND dav2d and that every existing general-intra fixture (D113/D135/D157, cardinal, SMOOTH, DC) stays byte-identical and a still-unsupported one-sided angle (D67/D203) rejects.
