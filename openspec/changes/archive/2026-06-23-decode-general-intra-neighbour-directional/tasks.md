## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-NEIGHBOUR-DIRECTIONAL` to the implementation matrix.
- [x] 1.2 Add the `general-intra-neighbour-directional` decoder support row.
- [x] 1.3 Add the `syn-rdir-intra-128x64-q80.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Add `reconstruct_general_intra_directional_neighbour_block_into` and `build_directional_middle_edges` (plane-general § 7.13.2.1 edge build from the partially-built frame + the shared § 7.13.2.8 bilinear middle-angle predictor, bit-exact for D135 by `shift == 0`).
- [x] 2.2 Admit a first-superblock-row, non-top-left, full 64x64 superblock D135 luma block over the real reconstructed left column and dispatch it to the new recon; keep row>0 / sub-superblock / `ctx != 0` / other-angle directional blocks rejected.
- [x] 2.3 Admit the `uv_mode == 0` directional-follow D135 chroma for the same neighbour-having block (routing `D135Follow` with `x > 0 || y > 0` to the neighbour path); keep neighbour-having row>0 chroma rejected.

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.
