## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-NONDC-LUMA-SMOOTH` to the implementation matrix.
- [x] 1.2 Add the `general-intra-nondc-luma-smooth` decoder support row.
- [x] 1.3 Add the `syn-vsmooth-intra-64x64-q120.ivf` and `syn-hsmooth-intra-64x64-q120.ivf` fixtures, conformance manifest entries, and reciprocal LOCAL-REFERENCE-EVIDENCE entries.

## 2. Implementation

- [x] 2.1 Map the reconstructed §9.2 luma mode to the supported non-DC predictor (`IntraYMode::supported_nondc`, `SupportedNonDcLumaMode`).
- [x] 2.2 Refactor the residual reconstruction to take a per-sample prediction buffer (`reconstruct_general_intra_block_with_prediction`); the DC path becomes the flat-prediction special case.
- [x] 2.3 Add `reconstruct_general_intra_luma_nondc_first_block_into`: build the §7.13.2.13 smooth prediction over the §7.13.2.1 no-neighbour fallback edges and add the §5.20.7.27 AC residual.
- [x] 2.4 Gate the block decode to DC chroma and the supported non-DC luma modes at the top-left (no-neighbour) block; reject everything else before reconstruction.

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.
