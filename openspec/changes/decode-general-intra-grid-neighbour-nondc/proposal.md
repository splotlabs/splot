## Why

The general intra decode reads a real reconstructed neighbour edge for a
§ 7.13.2.13 `SMOOTH_V`/`SMOOTH_H` luma block only in the FIRST superblock row
(`frontier.r == 0`, `haveAbove == 0`), where the § 7.13.2.1 above row is the
no-neighbour fallback and only the left column is real. A row>0 SMOOTH luma block
— which reads the real reconstructed above row `CurrFrame[0][y - 1][...]` and, for
a non-rightmost superblock, has a decoded above-right neighbour — was rejected
with the `general_intra_multirow_neighbour_non_dc` diagnostic. The SMOOTH chroma
grid path (`DECODE-GENERAL-INTRA-GRID`) already reads the real above row +
above-right sentinel bit-exact for row>0, and the luma neighbour reconstruction
delegates to the same plane-general edge builder + above-right resolver. Lifting
the first-row gate for SMOOTH luma unblocks a full 2-D grid of non-DC luma
superblocks.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-GRID-NEIGHBOUR-NONDC`.
- Lift the first-superblock-row gate on the § 7.13.2.13 SMOOTH (`SMOOTH_V` /
  `SMOOTH_H`) luma neighbour-edge decode: admit a full-superblock SMOOTH luma
  block at ANY 2-D grid position. The admission match collapses the prior
  first-row-admit / row>0-reject arms into one `n4w == FULL_SB_N4_LUMA` admit, and
  `nondc_luma_has_neighbour` drops its `&& frontier.r == 0`.
- Keep the reconstruction unchanged:
  `reconstruct_general_intra_luma_nondc_neighbour_block_into` already delegates to
  the plane-general `reconstruct_general_intra_smooth_over_edges_into`, which builds
  the § 7.13.2.1 edges (`haveAbove`/`haveLeft` per position) from the partially-built
  frame and resolves the top-right sentinel via `resolve_smooth_above_right_sentinel`
  over `full_sb_num4_above_right` (§ 5.20.7.25 `count_top_right_avail` over the
  § 5.20.2.3 `BlockDecoded` state) — the same machinery the SMOOTH chroma grid uses.
- Keep the § 8.3.2 `y_mode_index` ctx rejection intact: SMOOTH_V/H are
  non-directional (`modeDelta < NON_DIRECTIONAL_MODES_COUNT`) so ctx stays 0 and they
  are admitted; a directional neighbour (`ctx != 0`) is still rejected.
- Add the project-owned `syn-vgrid-intra-192x128-q120.ivf` fixture (a 3x2
  superblock grid whose middle, non-rightmost, row>0 superblock codes SMOOTH_V_PRED
  luma and reads a real reconstructed above row) and prove it decodes bit-exactly to
  the avmdec AND dav2d oracle, where the old code rejected the frame.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-grid-neighbour-nondc`: Crate-private general intra 2-D
  grid non-DC SMOOTH (`SMOOTH_V` / `SMOOTH_H`) luma decode over a real
  reconstructed above row at any superblock position.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra 2-D grid non-DC SMOOTH luma decode.

## Impact

- Affects `crates/splot-decode/src/runtime_minimal.rs` (the admission match and
  the `nondc_luma_has_neighbour` gate) and the doc comment on
  `crates/splot-decode/src/runtime_minimal_recon.rs`
  `reconstruct_general_intra_luma_nondc_neighbour_block_into` (now reachable for
  the row>0 above-row case). No new public surface; the recon code path is reused.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and
  generated status docs.
- No dependency graph, encoder, or validator changes. Directional luma over a real
  neighbour edge, the `ctx != 0` `y_mode_index` decode, SMOOTH/PAETH luma,
  sub-superblock (split) non-DC blocks, multiple tiles, inter prediction, and
  in-loop filters remain out of scope.
