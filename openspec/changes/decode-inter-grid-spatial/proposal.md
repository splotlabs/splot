## Why

The inter decoder decodes a MULTI-SUPERBLOCK inter frame, but only a single
superblock ROW (height 64) or column (width 64) of 64x64 superblocks
(`DECODE-INTER-MULTI-SB-SPATIAL`). Real content needs a full 2-D superblock GRID:
`ac0ej3` is 1920x1080, a 30x17 grid of 64x64 superblocks, so decoding a frame
whose width AND height are both greater than 64 is the next prerequisite for
real-content scale.

The recorded reason the single-SB-row brick rejected a 2-D grid was that "a
non-leftmost, non-top superblock's § 7.12.2 above-right / below-left scan
positions reach a not-yet-decoded superblock." Investigation shows the existing
`find_mv_stack` kernel already models that case correctly: the § 7.12.2.6 Scan
point process invokes the add-reference-MV step only when `is_inside(mvRow,
mvCol)` AND `RefFrames[mvRow][mvCol][0] has been written for this frame (this
checks that the candidate location has been decoded)`, and `NeighbourMvGrid::get`
returns `None` for an out-of-bounds position (`is_inside == 0`) OR an undecoded MI
cell (RefFrames not yet written). The § 5.20.2.1 raster loop decodes superblocks
in order and writes each block's MI cells before the next block's scan, so a probe
into a not-yet-decoded superblock reads an unwritten cell -> no candidate.

The smallest bit-exact-verifiable step is a two-frame 128x128 stream (a 2x2 grid
of 64x64 superblocks), each superblock a single 64x64 single-reference inter
block: SB0 is NEWMV (a non-zero MV) and SB2 / SB3 — in the SECOND superblock ROW —
are NEARMV that must predict SB0's MV ACROSS the superblock-row boundary from the
spatial-neighbour MV stack. Both oracles agree byte-for-byte.

## What Changes

- Add Feature ID `DECODE-INTER-GRID-SPATIAL`.
- Lift the § 5.18.3 inter frame-size gate in `validate_inter_frame_core` from a
  single superblock ROW/column to a full 2-D superblock GRID (width AND height
  each a positive multiple of 64), still gated to `seq_sb_size == 64x64`. This is
  the ONLY decoder change: the § 5.20.2.1 SB raster loop, the frame-wide
  `find_mv_stack` grid (already modelling § 7.12.2.6 availability via the
  `NeighbourMvGrid::get` "unwritten cell -> None" rule), `decode_inter_blocks`,
  and the tile-payload boundary derivation were already geometry-agnostic and are
  unchanged.
- Add the project-owned `syn-grid-inter-128x128-q80.ivf` fixture (frame 0 = four
  DC_PRED intra 64x64 superblocks; frame 1 = four 64x64 single-reference inter
  blocks, all skip=1, SB0 NEWMV + SB1/SB2/SB3 NEARMV reusing SB0's MV, SB2/SB3
  across the superblock-row boundary). Prove avmdec `--rawvideo --i420` and dav2d
  `--demuxer ivf` agree byte-for-byte (md5 `897bf67e72ec04cb7275fae08eab700c`,
  49152 bytes).
- Register the fixture in the conformance manifest (`expect = "clean"`) and add
  the reciprocal LOCAL-REFERENCE-EVIDENCE entry.
- Add decode tests pinning the bit-exact output (per-frame hash + the CLI raw
  output round-trip) and `find_mv_stack` unit tests proving the cross-SB-row
  availability (a second-SB-row block predicts the above SB's MV; an undecoded
  later-column SB yields no candidate).

## Capabilities

### New Capabilities
- `decode-inter-grid-spatial`: A 2-D-grid inter frame (width and height each a
  positive multiple of 64) decodes bit-exact, with a block in a later superblock
  row predicting its motion vector from an already-decoded superblock in an
  earlier row across the superblock-row boundary via the frame-wide § 7.11/§ 7.12
  spatial MV-stack process and the § 7.12.2.6 decoded-location availability gate.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row.

## Impact

- Adds `tests/conformance/vectors/valid/syn-grid-inter-128x128-q80.ivf` and decode
  tests in `crates/splot-decode/src/runtime_minimal/inter/tests.rs`,
  `crates/splot-decode/src/runtime_minimal/inter/find_mv_stack/tests.rs`, and
  `crates/splot-cli/tests/decode_cli.rs`.
- Changes only the § 5.18.3 frame-size gate in
  `crates/splot-decode/src/runtime_minimal/inter.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and the
  generated status/coverage docs.
- No public API, dependency graph, encoder, or validator changes. A partial
  (non-multiple-of-64) frame size, a multi-superblock skip == 0 residual, and the
  deferred temporal / compound / warp / ref-MV-bank / derived-SMVP / DRL-reorder
  MV candidates remain out of scope (rejected with a structured diagnostic before
  any output).
