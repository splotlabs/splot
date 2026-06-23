## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-DIRECTIONAL-FOLLOW-CHROMA` to the implementation matrix.
- [x] 1.2 Add the `general-intra-directional-follow-chroma` decoder support row.
- [x] 1.3 Add the `syn-dfchroma-intra-64x64-q80.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Add `SupportedChromaMode::D135Follow` and resolve `UVMode == D135_PRED` to it in `supported_chroma_mode`, only for the directional-follow branch (`uv_mode == 0` and the luma is directional).
- [x] 2.2 Reconstruct `D135Follow` chroma via `reconstruct_general_intra_chroma_directional_first_into` (the § 7.13.2.8 middle-angle prediction over the § 7.13.2.1 no-neighbour fallback edges plus the § 5.20.7.27 residual).
- [x] 2.3 Gate `D135Follow` to the top-left (no-neighbour) 64x64 superblock; reject a neighbour-having directional chroma block (`general_intra_directional_chroma_neighbour`) and keep CfL/CCTX/MHCCP chroma rejected.

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.
