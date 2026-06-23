## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-GRID-NEIGHBOUR-NONDC` to the implementation matrix.
- [x] 1.2 Add the `general-intra-grid-neighbour-nondc` decoder support row.
- [x] 1.3 Add the `syn-vgrid-intra-192x128-q120.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Lift the first-superblock-row gate: admit a full-superblock §7.13.2.13 SMOOTH (`SMOOTH_V`/`SMOOTH_H`) luma block at any 2-D grid position (collapse the first-row-admit / row>0-reject arms into one `n4w == FULL_SB_N4_LUMA` admit).
- [x] 2.2 Drop `&& frontier.r == 0` from `nondc_luma_has_neighbour` so a row>0 SMOOTH luma block uses the neighbour-edge reconstruction (real above row + above-right resolver).
- [x] 2.3 Keep the directional-neighbour `y_mode_index` ctx rejection (`general_intra_directional_neighbour_y_mode_index_ctx`) and the sub-superblock non-DC rejection intact.

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.
