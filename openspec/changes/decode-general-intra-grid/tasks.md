## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-GRID` to the implementation matrix.
- [x] 1.2 Add the `general-intra-grid` decoder support row.
- [x] 1.3 Add the `syn-grid-intra-128x128-q80.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Add the total/checked `CurrentFrameWorkspace::reconstructed_sample` accessor (splot-recon).
- [x] 2.2 Derive §7.13.2.1 `num4AboveRight` faithfully to §5.20.7.25 `count_top_right_avail` for the full-superblock chroma case (`full_sb_chroma_num4_above_right`).
- [x] 2.3 Read the §7.13.2.13 SMOOTH chroma top-right sentinel `AboveRow[w]` from the real reconstructed above-right sample when decoded (`resolve_smooth_above_right_sentinel`).
- [x] 2.4 Relax `is_general_minimal_intra` to accept width and height both positive multiples of 64 (a full 2-D grid of 64x64 superblocks).

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.
