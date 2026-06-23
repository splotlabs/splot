## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-IDIF-D157` to the implementation matrix and update `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION` (now implements the luma IDIF 4-tap).
- [x] 1.2 Add the `general-intra-idif-d157` decoder support row.
- [x] 1.3 Add the `syn-d157-intra-128x64-q80.ivf` fixture, its conformance manifest entry, and the reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Add the § 7.13.2.8 / § 9.2 `Dr_Interp_Filter[32][4]` table (verbatim from the spec mirror) and the `enableIdif == 1` luma IDIF 4-tap path (`predict_intra_middle_directional_angle_rect_idif_into`, `IntraMiddleDirectionalAngleIdifEdges`) in `splot-recon`, computing `s = Σ(t=0..3) Dr_Interp_Filter[shift][t] * Edge[base + t - 1]; pred = Clip1(Round2(s, 7))` over the wider `Edge[-2..=side+1]` edges, with the signed sum floored by Round2 and Clip1 clamping negatives to 0. Verify D135 (`shift == 0`) stays a sample copy.
- [x] 2.2 Lift the `splot-recon` workspace luma rejection for the middle directional-angle path: dispatch on plane (luma → IDIF via the § 7.13.2.8 edge extension `Edge[-2] = Edge[-1]`, `Edge[side] = Edge[side+1] = Edge[side-1]`; chroma → bilinear).
- [x] 2.3 Map § 9.2 mode value 6 to `SupportedDirectionalLumaMode::D157` and `uv_mode == 0` follow to `SupportedChromaMode::D157Follow`; route the neighbour-having luma D157 block to the IDIF 4-tap (plane-dispatched in `reconstruct_general_intra_directional_neighbour_block_into`) and the follow chroma to the bilinear branch; admit only the first-superblock-row, non-first-column position; reject the top-left / first-column / sub-superblock / row>0 D157 positions and D113/D45/D67/D203.

## 3. Documentation And Verification

- [x] 3.1 Add the splot-recon IDIF unit/property tests (4-tap formula, `shift == 0` == copy, IDIF != bilinear at nonzero shift, negative-sum clamp) and the D157 decode-to-oracle test, and regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, conformance, and the Rust acceptance gate; confirm the D157 fixture decodes bit-exact vs avmdec AND dav2d.
