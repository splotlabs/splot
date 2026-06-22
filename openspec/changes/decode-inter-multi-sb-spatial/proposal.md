## Why

The inter decoder decodes a multi-block frame but still inside a single 64x64
superblock (`DECODE-INTER-MVSTACK-SPATIAL`). Real content needs MULTI-SUPERBLOCK
inter frames: `ac0ej3` is 1920x1080, so decoding more than one 64x64 superblock
is a prerequisite for real-content scale. The AV2 entry point is the § 5.20.2.1
`decode_tile()` superblock raster loop (already in the shared partition walker)
plus the frame-wide § 7.11/§ 7.12 `find_mv_stack` grid, so a block in a later
superblock predicts its motion vector from the immediately-prior superblock's
reconstructed-edge neighbour across the superblock boundary.

The smallest bit-exact-verifiable step is a two-frame 128x64 stream (two
horizontally-adjacent 64x64 superblocks), each superblock a single 64x64
single-reference inter block: SB0 is NEWMV (a non-zero MV) and SB1 — in the
second superblock — is NEARMV that must predict SB0's MV across the superblock
boundary from the spatial-neighbour MV stack. Both oracles agree byte-for-byte.

## What Changes

- Add Feature ID `DECODE-INTER-MULTI-SB-SPATIAL`.
- Lift the § 5.18.3 inter frame-size gate in `validate_inter_frame_core` from
  exactly 64x64 to a single superblock ROW (height 64, width a positive multiple
  of 64) OR single superblock COLUMN (width 64, height a positive multiple of 64)
  of 64x64 superblocks. This is the only decoder change: the § 5.20.2.1 SB raster
  loop, the frame-wide `find_mv_stack` grid, `decode_inter_blocks`, and the
  tile-payload boundary derivation were already geometry-agnostic and are
  unchanged.
- Add the project-owned `syn-2sb-inter-128x64-q80.ivf` fixture (frame 0 = two
  DC_PRED intra 64x64 superblocks; frame 1 = two 64x64 single-reference inter
  blocks, SB0 NEWMV + SB1 NEARMV reusing SB0's MV across the superblock boundary,
  both skip=1). Prove avmdec `--rawvideo --i420` and dav2d `--demuxer ivf` agree
  byte-for-byte (md5 `477a993d671e93d37b92a0d368c238ff`, 24576 bytes).
- Register the fixture in the conformance manifest (`expect = "clean"`) and add
  the reciprocal LOCAL-REFERENCE-EVIDENCE entry.
- Add decode tests pinning the bit-exact output (per-frame hash + the CLI raw
  output round-trip).

## Capabilities

### New Capabilities
- `decode-inter-multi-sb-spatial`: A multi-superblock inter frame (a single
  superblock row or column of 64x64 superblocks) decodes bit-exact, with a block
  in a later superblock predicting its motion vector from the immediately-prior
  superblock's reconstructed-edge neighbour across the superblock boundary via the
  frame-wide § 7.11/§ 7.12 spatial MV-stack process.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row.

## Impact

- Adds `tests/conformance/vectors/valid/syn-2sb-inter-128x64-q80.ivf` and decode
  tests in `crates/splot-decode/src/runtime_minimal/inter/tests.rs` and
  `crates/splot-cli/tests/decode_cli.rs`.
- Changes only the § 5.18.3 frame-size gate in
  `crates/splot-decode/src/runtime_minimal/inter.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and the
  generated status/coverage docs.
- No public API, dependency graph, encoder, or validator changes. A full 2-D
  superblock grid (both dimensions > 64), a multi-superblock skip == 0 residual,
  and the deferred temporal / compound / warp / ref-MV-bank / derived-SMVP /
  DRL-reorder MV candidates remain out of scope (rejected with a structured
  diagnostic before any output).
