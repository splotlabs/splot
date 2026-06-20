## Why

The general intra decode path decodes a single 64x64 block end-to-end
bit-exactly. The next step toward decoding real AV2 intra frames — which are
almost always partitioned — is to walk the full § 5.20.3.1 partition tree and
decode every leaf block, with the neighbour-dependent prediction and contexts
that multi-block decode requires.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-MULTIBLOCK`.
- Add `decode_general_intra_partition_tree`: a depth-first walk of the complete
  § 5.20.3.1 partition tree that reads partition-split symbols and per-block
  syntax interleaved on one live symbol decoder and the tile CDFs, invoking a
  per-leaf callback in decode order and maintaining the § 5.20.4.1 MI-size
  partition context across blocks.
- Add `decode_general_intra_plane_coeffs`: a per-block § 5.20.7.27 coefficient
  decode that derives the § 8.3.2 `txb_skip` context from the persistent
  `TileCoeffContextState` neighbour lines (and commits the zero-block context),
  threading one persistent context across all blocks so `dc_sign` and the
  per-block context updates are neighbour-aware.
- Reconstruct each leaf block into a persistent `CurrentFrameWorkspace` in decode
  order via `reconstruct_general_intra_block_into`, so each non-first block's
  § 7.13.2 DC prediction reads its already-reconstructed above/left neighbours
  (`128` fallback when none).
- Unify the single-block general intra decode through this driver and validate
  § 8.2.4 `exit_symbol()` after the whole tile.
- Gate to DC_PRED square blocks (mode contexts reduce to the tile-origin ctx 0);
  reject non-DC modes and non-square (rectangular leaf) partitions.
- Add the project-owned `syn-quad-intra-64x64-q80.ivf` fixture (four flat
  quadrants splitting into four square 32x32 DC blocks) and prove it decodes
  bit-exactly to the avmdec/dav2d oracle.
- Update decoder tracking, roadmap, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-multiblock`: Crate-private split-partition multi-block
  general intra decode with neighbour DC prediction and neighbour coefficient
  contexts.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra multi-block partition decode.

## Impact

- Affects `crates/splot-decode/src/tile_payload/partition_traversal.rs`,
  `crates/splot-decode/src/tile_payload/runtime_frontier.rs`,
  `crates/splot-decode/src/tile_payload/general_intra_residual.rs`,
  `crates/splot-decode/src/tile_payload.rs`,
  `crates/splot-decode/src/runtime_minimal.rs`, and
  `crates/splot-decode/src/runtime_minimal_recon.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, `docs/DECODER-ROADMAP.md`, and
  generated status/coverage docs.
- No public API, dependency graph, encoder, or validator changes. Non-DC modes,
  rectangular-leaf partitions, multiple tiles, non-64x64 frames, chroma
  `cctx`/CfL, inter prediction, in-loop filters, and live in-CI AVM/dav2d remain
  out of scope.
