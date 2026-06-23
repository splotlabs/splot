## Why

> **Historical note.** This change predates `decode-minimal-fixture-avm-skip-polarity`, which retired the frozen `base_q_idx == 255` committed minimal fixture (`syn-flat-intra-64x64-minimal.ivf`) and replaced it with an AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream that routes through the general intra path. References below to the committed minimal fixture as the frozen `base_q_idx == 255` anchor — and to keeping that committed fixture's hash byte-identical — are historical; the routing rule (a `base_q_idx == 255` frame falls through to the frozen gate) still holds.

The general intra decode path decodes the § 5.20.5.3 block mode info and the
§ 5.20.7.27 luma transform-block coefficients, then stops at chroma. The final
step toward decoding a real AVM-generated minimal-tool intra frame — and the
first end-to-end bit-exact frame decode — is to decode the chroma coefficients,
dequantize / inverse-transform / residual-add every plane over the no-neighbour
DC prediction, and assemble the reconstructed frame, then prove it matches the
avmdec/dav2d oracle byte-for-byte.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-FRAME-RECON`.
- Add `decode_general_intra_chroma_coeffs` decoding the U then V 32x32 chroma
  transform blocks. The `all_zero` (`txb_skip`) symbol uses the § 8 parsing CDF
  `TileTxbSkipCdf[is_inter || fsc_mode][txSzCtx][ctx]` for plane 0/1 (the second
  index is `is_inter || fsc_mode`, NOT plane_type; the U-plane offset lives in
  `ctx == (above != 0) + (left != 0) + 6`) and `TileVTxbSkipCdf[ctx]` for plane 2
  (`ctx == (EobU != 0) ? 6 : 0`); the nonzero pass reuses the existing
  coefficient-loop entry with the chroma plane.
- Generalize the luma reconstruction into `reconstruct_general_intra_block`,
  composing the § 7.14.4 dequantization (the `dqDenom` TCQ term applying to the
  luma DCT_DCT block only), the § 7.15.4 inverse transform (64x64 via the
  adjusted 32x32 inverse plus duplication for luma, native 32x32 for chroma),
  and the § 7.14.3 residual add over the § 7.13.2 no-neighbour DC prediction.
- Validate § 8.2.4 `exit_symbol()` after the coefficients and assemble the
  8-bit 4:2:0 frame from the three reconstructed planes
  (`assemble_general_intra_frame`).
- Wire it into `decode_general_minimal_intra_frame` so the committed
  `syn-flat-intra-64x64-q80.ivf` fixture decodes to a full reconstructed frame
  instead of an unsupported-feature diagnostic.
- Keep the frozen `base_q_idx == 255` minimal hash contract byte-identical.
- Replace the prior "reaches chroma" CLI test with one asserting the bit-exact
  full-frame decode, and add tests pinning the reconstructed flat planes and the
  decoded-frame hash.
- Update decoder tracking, roadmap, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-frame-recon`: Crate-private AV2 chroma coefficient decode
  plus § 7.14.4 / § 7.15.4 / § 7.14.3 reconstruction and frame assembly for the
  general intra path, verified bit-exactly against the avmdec/dav2d oracle.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra full frame reconstruction.

## Impact

- Affects `crates/splot-decode/src/tile_payload/general_intra_residual.rs`,
  `crates/splot-decode/src/tile_payload.rs`,
  `crates/splot-decode/src/runtime_minimal.rs`,
  `crates/splot-decode/src/runtime_minimal_recon.rs`, and
  `crates/splot-cli/tests/decode_cli.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, `docs/DECODER-ROADMAP.md`, and
  generated status/coverage docs.
- No public API, dependency graph, encoder, or validator changes. Split
  partitions, multiple blocks, multiple tiles, non-64x64 frames, chroma
  `cctx`/CfL, inter prediction, in-loop filters, and live in-CI AVM/dav2d
  integration remain out of scope.
