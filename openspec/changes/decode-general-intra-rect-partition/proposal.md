## Why

The general intra decode path handles square partitions only: every committed
fixture (DC, SMOOTH, directional) is a square 64x64 / 32x32 / 16x16 block, and a
rectangular partition leaf (`n4w != n4h`, the PARTITION_HORZ / PARTITION_VERT
family) is rejected up front with `general_intra_non_square_block`. Real AV2
intra frames are heavily rectangular, so the next strategic partition increment
is the first rectangular leaf decode. The smallest verifiable step is a 64x64
superblock the encoder splits via PARTITION_HORZ into two RECTANGULAR 64x32
DC_PRED leaves.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-RECT-PARTITION`.
- Decode and reconstruct a rectangular (non-square) general intra leaf, gated to
  the verified DC_PRED luma + DC chroma subset. The § 7.13.2.4 DC predictor reads
  only the immediate in-frame left column / above row from the persistent
  workspace (no § 7.13.2.1 above-right / below-left sentinels, so no § 5.20.2.3
  `BlockDecoded` state is needed), so a rectangular DC leaf reconstructs
  correctly at any superblock position.
- Generalise the § 5.20.7.27 `coeffs()` general-intra context spans to read the
  transform width (`Tx_Width[txSz] >> 2`) and height (`Tx_Height[txSz] >> 2`)
  independently from the generated § 9.2 conversion tables (square is the
  `w4 == h4` special case). The nonzero coefficient geometry (scan, eob class,
  dequant, transform) already reads width and height independently.
- Add a rectangular DC reconstruction helper
  (`reconstruct_general_intra_block_rect` / `_rect_into`) that composes the
  § 7.14.4 dequantization, the § 7.15.4 inverse transform (including the
  § 7.15.4.1 √2 rescale for an odd `|log2_w - log2_h|` ratio), and the § 7.14.3
  residual add over the rectangular flat DC prediction. The existing rectangular
  `IntraRectBlockSize` / `intra_dc_edges_for_rect` / `predict_intra_dc_rect_value`
  / `write_rect_block` primitives already accept rectangular sizes.
- Derive the rectangular transform size (`Max_Tx_Size_Rect` under
  TX_MODE_LARGEST) by mapping the block width/height log2 to its § 9.2 `TX_SIZES_ALL`
  index via the conversion tables (TX_64X32 luma, TX_32X16 chroma); both resolve
  to `DCT_DCT` (§ 5.20.8.2 `get_tx_set` returns TX_SET_DCTONLY for
  `txSzSqrUp >= TX_32X32`).
- Add the project-owned `syn-hrect-intra-64x64-q120.ivf` fixture and prove it
  decodes bit-exactly to the avmdec/dav2d oracle.
- Add a decode test pinning the per-band DC values and the frame hash; confirm
  all existing general intra fixtures still decode bit-exact and that a non-DC
  rectangular leaf (SMOOTH / directional luma or non-DC chroma) still rejects.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-rect-partition`: A rectangular (non-square) DC_PRED
  partition leaf decode (PARTITION_HORZ / PARTITION_VERT family), reconstructed
  bit-exact via the rectangular transform / DC prediction path.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra rectangular partition decode.

## Impact

- Adds `tests/conformance/vectors/valid/syn-hrect-intra-64x64-q120.ivf` and a
  decode test in
  `crates/splot-decode/src/runtime_minimal/general_intra_tests.rs`.
- Modifies `crates/splot-decode/src/runtime_minimal/general_intra.rs`,
  `crates/splot-decode/src/runtime_minimal_recon.rs`,
  `crates/splot-decode/src/tile_payload/general_intra_residual.rs`, and the
  `tile_payload` re-exports.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, and generated status/coverage
  docs.
- No public API, dependency graph, encoder, or validator changes. Non-DC
  rectangular luma / chroma prediction (which would need rectangular § 7.13.2.8 /
  § 7.13.2.13 predictors and the § 5.20.2.3 above-right / below-left sentinels),
  non-64x64 frames, inter prediction, in-loop filters, and live in-CI AVM/dav2d
  remain out of scope.
