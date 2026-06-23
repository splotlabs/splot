## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-DIRECTIONAL-CORNER` to the implementation matrix.
- [x] 1.2 Add the `general-intra-directional-corner` decoder support row.
- [x] 1.3 Add the `syn-d135row-intra-128x128-q80.ivf` fixture, its conformance manifest entry, and the reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Build the real § 7.13.2.1 corner in `build_directional_middle_edges`: the `haveLeft && haveAbove` arm uses the caller-supplied `CurrFrame[plane][y-1][x-1]` (read via `reconstructed_sample`) plus the real above row + left column; the `!haveLeft && haveAbove` arm uses the real above row and derives `LeftCol[i] == AboveRow[-1] == AboveRow[0]`. Keep `(true, false)` / `(false, false)` byte-identical.
- [x] 2.2 Read the corner in `reconstruct_general_intra_directional_neighbour_block_into` only for the `haveLeft && haveAbove` arm, thread the real above samples through, and run the existing plane-dispatched IDIF (luma) / bilinear (chroma) middle-angle kernel + § 5.20.7.27 residual.
- [x] 2.3 Admit the row>0, non-first-column full-superblock D135 luma block (`frontier.r != 0 && frontier.c != 0 && n4w == 16`) and its `uv_mode == 0` directional-follow D135 chroma; keep the row>0 first-column / row>0 D157 / sub-superblock / non-zero-delta / directional-neighbour-reorder positions rejected with structured `decode/unsupported-feature` diagnostics.

## 3. Documentation And Verification

- [x] 3.1 Add the row>0 directional corner decode-to-oracle test and regenerate the feature/status/support/coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, conformance, and the Rust acceptance gate; confirm the fixture decodes bit-exact vs avmdec AND dav2d and that every existing general-intra fixture (especially every D135/D157 and the no-neighbour / first-row directional) stays byte-identical.
