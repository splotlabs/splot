## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-ANGLE-D113` to the implementation matrix.
- [x] 1.2 Add the `general-intra-angle-d113` decoder support row.
- [x] 1.3 Add the `syn-d113-intra-128x128-q80.ivf` fixture, its conformance manifest entry, and the reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Map mode value 5 to `SupportedDirectionalLumaMode::D113` (`IntraYMode::supported_directional`) and pAngle 113 to `IntraMiddleDirectionalAngle::D113` (`middle_directional_angle`); add `SupportedChromaMode::D113Follow` and its § 5.20.5.3 `uv_mode == 0` directional-follow resolution.
- [x] 2.2 Route the neighbour-having luma D113 block and the D113-follow chroma through the existing plane-dispatched `reconstruct_general_intra_directional_neighbour_block_into` (luma § 7.13.2.8 IDIF 4-tap, chroma `enableIdif == 0` bilinear) + the real § 7.13.2.1 corner builder + the § 5.20.7.27 residual.
- [x] 2.3 Admit ONLY the row>0, non-first-column full-superblock D113 luma block (`frontier.r != 0 && frontier.c != 0 && n4w == 16`) and its `uv_mode == 0` directional-follow D113 chroma; keep the top-left / first-row / first-column / sub-partitioned / non-64x64 D113 positions, the one-sided angles D45/D67/D203, non-zero angle deltas, and the directional-neighbour (`ctx != 0`) escape reorder rejected with structured `decode/unsupported-feature` diagnostics.

## 3. Documentation And Verification

- [x] 3.1 Add the D113 vertical-leaning IDIF decode-to-oracle test and the supporting unit tests (`supported_directional` admits D113; the `y_mode_offset` escape reconstructs D113), and regenerate the feature/status/support/coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, conformance, and the Rust acceptance gate; confirm the fixture decodes bit-exact vs avmdec AND dav2d and that every existing general-intra fixture (especially every D135/D157, cardinal, and SMOOTH) stays byte-identical and a still-unsupported angle (D45) rejects.
