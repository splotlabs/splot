## Why

> **Historical note.** This change predates `decode-minimal-fixture-avm-skip-polarity`, which retired the frozen `base_q_idx == 255` committed minimal fixture (`syn-flat-intra-64x64-minimal.ivf`) and replaced it with an AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream that routes through the general intra path. References below to the committed minimal fixture as the frozen `base_q_idx == 255` anchor — and to keeping that committed fixture's hash byte-identical — are historical; the routing rule (a `base_q_idx == 255` frame falls through to the frozen gate) still holds.

The general intra decode path reconstructs a single 64x64 superblock
bit-exactly (DC, multi-coefficient, split-partition multi-block, single-block
non-DC smooth). Real AV2 frames are larger than one superblock. The next step is
to generalize the tile traversal from one superblock to every superblock in the
tile's MI range per AV2 § 5.20.2.1 `decode_tile()`, so that multi-superblock
frames whose width and height are positive multiples of 64 (here 128x64) decode
bit-exactly, with the right superblock DC-predicting from the reconstructed left
neighbour.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-NON64-MULTISB`.
- Relax the IVF size gate (`require_minimal_ivf`) to admit any positive-sized
  AV02 single-frame container to routing (keeping AV02, `frame_count == 1`, one
  in-memory frame, and no container warnings). The frozen `base_q_idx == 255`
  hash tier still re-imposes its strict 64x64 requirement in
  `validate_frame_core`.
- Accept frame sizes that are positive multiples of 64 (width and height) in
  `is_general_minimal_intra`.
- Parameterize `new_general_intra_workspace` by the real frame size (chroma =
  half for 4:2:0), threaded from the parsed `core.frame_size`.
- Iterate every superblock in the tile's MI range in raster order per AV2
  § 5.20.2.1 `decode_tile()` (`sbSize4 = Num_4x4_Blocks_Wide[SbSize]` MI steps;
  `clear_left_context()` at the start of each superblock row) in
  `decode_general_intra_partition_tree`, reusing one `SymbolDecoder`, the tile
  CDFs, the `TileMiSizeState` partition context, and the frame-spanning
  `TileCoeffContextState` and reconstruction workspace.
- Add § 7.13.2.13 `SMOOTH_PRED` chroma reconstruction (resolved from `uv_mode`
  via § 5.20.5.3 `get_intra_uv_mode_set` for the non-directional luma subset)
  over § 7.13.2.1 neighbour edges read from the partially-built frame, since the
  multi-superblock encoder codes the second superblock's (residual-free) chroma
  as `SMOOTH_PRED`.
- Add the project-owned `syn-2sb-intra-128x64-q80.ivf` fixture and prove it
  decodes bit-exactly to the avmdec and dav2d oracle, with existing 64x64
  fixtures still bit-exact.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-non64-multisb`: Crate-private multi-superblock
  (non-64x64, positive multiple of 64) general intra DC decode, iterating the
  AV2 § 5.20.2.1 superblock grid and reconstructing DC luma plus DC / SMOOTH
  chroma over reconstructed neighbours.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra multi-superblock (non-64x64) DC decode.

## Impact

- Affects `crates/splot-decode/src/runtime_minimal.rs`,
  `crates/splot-decode/src/runtime_minimal_recon.rs`,
  `crates/splot-decode/src/tile_payload/partition_traversal.rs`,
  `crates/splot-decode/src/tile_payload/mi_size_state.rs`,
  `crates/splot-decode/src/tile_payload/general_intra_block.rs`,
  `crates/splot-decode/src/tile_payload/cdf/block_context.rs`, and
  `crates/splot-decode/src/tile_payload.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`, and generated status docs.
- No public API, dependency graph, encoder, or validator changes. Partial frames
  (non-multiple-of-64 sizes needing edge clamping), non-DC/non-SMOOTH chroma,
  multiple tiles, inter prediction, in-loop filters, and live in-CI AVM/dav2d
  remain out of scope.
