## Why

The general intra decode path proves DC blocks split arbitrarily deep
(`syn-deep`), but that brick explicitly deferred the AV2 § 5.20.2.3 per-block
`BlockDecoded` flag state: a DC predictor reads only the immediate left/above so
its availability is frame-position-based, but a SMOOTH (or directional) block
reads the § 7.13.2.1 above-right `AboveRow[w]` / below-left `LeftCol[h]`
sentinels, whose availability § 7.13.2.1 derives from `BlockDecoded` via
§ 5.20.7.25 `count_top_right_avail` / `count_bottom_left_avail`. Without a real
`BlockDecoded` grid, a sub-partitioned (SPLIT-child) SMOOTH block could not read
its intra-superblock sibling, so it was rejected. This is the "real unlock":
modelling `BlockDecoded` lets a split SMOOTH sub-block read the correct sentinel.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-BLOCK-DECODED`.
- Add the crate-private `TileBlockDecodedState`
  (`crates/splot-decode/src/tile_payload/block_decoded_state.rs`): a
  superblock-relative per-plane § 5.20.2.3 `BlockDecoded` grid with
  `clear_superblock` (§ 5.20.2.3 `clear_block_decoded_flags`),
  `count_top_right_avail` / `count_bottom_left_avail` (§ 5.20.7.25), and
  `set_block` (§ 5.20.4 per-block `BlockDecoded[plane][...] = 1`).
- Wire the grid into the § 5.20.3.1 partition walk
  (`decode_general_intra_partition_tree`): create it, clear it at each
  § 5.20.2.1 superblock, pass it read-only to the leaf callback, and mark each
  decoded transform block's plane 4x4 units after the leaf returns.
- Derive the luma § 7.13.2.1 `num4AboveRight` from `count_top_right_avail` over
  the real grid (new `luma_num4_above_right_from_block_decoded`) instead of the
  full-superblock-only `full_sb_num4_above_right` approximation, so a SPLIT
  child's above-right sibling is counted.
- Lift the prior `general_intra_multiblock_non_dc_subblock` reject for a
  SMOOTH_H luma SPLIT sub-block of size >= 32x32 (TX_SET_DCTONLY) reading a real
  reconstructed above-right sentinel. SMOOTH_V luma sub-blocks (below-left
  sentinel) and SMOOTH chroma sub-blocks still reject with structured
  `decode/unsupported-feature` diagnostics.
- Add the project-owned `syn-shsplit-intra-64x64-q80.ivf` fixture (a SPLIT
  64x64 superblock whose bottom-left 32x32 SMOOTH_H block reads the decoded
  top-right sibling, 210, NOT the edge clamp, ~50) and prove it decodes
  bit-exactly to the avmdec/dav2d oracle.
- Add the negative `syn-svsplit-intra-64x64-q140.ivf` fixture (validates clean,
  oracles agree) that splot still rejects (`general_intra_smooth_chroma_subblock`)
  to pin the verified-subset boundary.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-block-decoded`: A § 5.20.2.3 per-block `BlockDecoded`
  state that lets a SMOOTH_H luma SPLIT sub-block read its § 7.13.2.1 above-right
  sentinel from an already-decoded intra-superblock sibling.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra per-block `BlockDecoded` SMOOTH_H above-right decode.

## Impact

- Adds `crates/splot-decode/src/tile_payload/block_decoded_state.rs`, the two
  fixtures `tests/conformance/vectors/valid/syn-shsplit-intra-64x64-q80.ivf` and
  `tests/conformance/vectors/valid/syn-svsplit-intra-64x64-q140.ivf`, and decode
  tests in `crates/splot-decode/src/runtime_minimal/general_intra_tests.rs`.
- Modifies `crates/splot-decode/src/tile_payload/partition_traversal.rs`,
  `crates/splot-decode/src/tile_payload/runtime_frontier.rs`,
  `crates/splot-decode/src/tile_payload.rs`, and
  `crates/splot-decode/src/runtime_minimal/general_intra.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, and generated status/coverage
  docs.
- No public API, dependency graph, encoder, or validator changes. SMOOTH_V
  below-left sub-block sentinels, SMOOTH chroma sub-blocks, directional (D135)
  sub-blocks, non-DCTONLY-size (<32x32) non-DC sub-blocks, non-64x64-grid
  constraints beyond the existing subset, inter prediction, in-loop filters, and
  live in-CI AVM/dav2d remain out of scope.
