## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-NON64-MULTISB` to the implementation matrix.
- [x] 1.2 Add the `general-intra-non64-multisb` decoder support row.
- [x] 1.3 Add the `syn-2sb-intra-128x64-q80.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Relax `require_minimal_ivf` to admit any positive-sized AV02 single-frame container to routing (keeping AV02, frame_count == 1, one in-memory frame, no warnings); the frozen tier re-imposes 64x64 in `validate_frame_core`.
- [x] 2.2 Accept positive-multiple-of-64 frame sizes in `is_general_minimal_intra`.
- [x] 2.3 Parameterize `new_general_intra_workspace` by the real frame size (chroma = half for 4:2:0), threaded from `core.frame_size`.
- [x] 2.4 Iterate every superblock in the tile's MI range in raster order per AV2 §5.20.2.1 `decode_tile()` (`sbSize4` steps; `clear_left_context()` per superblock row) in `decode_general_intra_partition_tree`, reusing one symbol decoder, tile CDFs, MI-size state, coeff context, and workspace.
- [x] 2.5 Add §7.13.2.13 SMOOTH_PRED chroma reconstruction (resolved via §5.20.5.3 `get_intra_uv_mode_set`) over §7.13.2.1 neighbour edges read from the partially-built frame; keep DC chroma and reject other non-DC chroma modes.

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.
