# Tasks

## 1. Investigation (decide if this is a clean single brick)
- [x] 1.1 Confirm `find_mv_stack` already models § 7.12.2.6 availability
      (`is_inside` && RefFrames-written) via `NeighbourMvGrid::get` returning
      `None` for an out-of-bounds or undecoded MI cell, so a probe into a
      not-yet-decoded superblock contributes no candidate.
- [x] 1.2 Confirm the § 5.20.2.1 raster loop (sb_row outer, sb_col inner) decodes
      every superblock in order and `record_block` writes before the next block's
      scan, so a later SB row's above probe lands in the decoded earlier row.

## 2. Fixture (verify oracles first)
- [x] 2.1 Generate `syn-grid-inter-128x128-q80.ivf` from a project-owned synthetic
      Y4M (128x128: four flat 64x64 luma superblocks 100/150/80/200 + flat chroma;
      frame 1 shifted left 6 luma samples, edge-clamped) at `--qp 80 --sb-size 64
      --min-partition-size 64 --max-partition-size 64` with broad decode tools
      disabled. Confirm the encode is deterministic (byte-identical re-encode).
- [x] 2.2 Confirm avmdec `--rawvideo --i420` == dav2d `--demuxer ivf --muxer yuv`
      byte-for-byte (md5 `897bf67e72ec04cb7275fae08eab700c`, 49152 bytes).
- [x] 2.3 Confirm via splot instrumentation that the inter frame is four 64x64
      superblocks: SB0 @ MI(0,0) NEWMV (col 48, has_neighbour=false), and SB1 @
      MI(0,16) / SB2 @ MI(16,0) / SB3 @ MI(16,16) NEARMV (col 48, NewMvContext 3,
      has_neighbour=true). SB2 and SB3 are in the SECOND superblock ROW and predict
      SB0's MV across the superblock-row boundary (not the zero fallback).
- [x] 2.4 Register in the conformance manifest + reciprocal
      LOCAL-REFERENCE-EVIDENCE entry.

## 3. Lift the frame-size gate (the only decoder change)
- [x] 3.1 Lift the § 5.18.3 `validate_inter_frame_core` frame-size gate from a
      single superblock row/column to a full 2-D superblock grid (width and height
      each a positive multiple of 64).
- [x] 3.2 Keep `seq_sb_size == 64x64` and every existing gate (>= 32x32 leaf,
      single-reference, no residual-tools, multi-SB skip==0 residual rejected,
      enable_bawp / flex_mvres / adaptive_mvd rejected, SWITCHABLE-interp /
      mv-stack-tools rejected once a neighbour exists).
- [x] 3.3 Confirm the § 5.20.2.1 SB raster loop, the frame-wide `find_mv_stack`
      grid, `decode_inter_blocks`, and the tile-payload boundary derivation need no
      change (geometry-agnostic; availability already modelled).

## 4. Verify + gate
- [x] 4.1 `splot decode syn-grid-inter-128x128-q80.ivf --output-format raw` ==
      oracle md5 byte-for-byte.
- [x] 4.2 Prove the OLD code rejected the 128x128 fixture
      (`inter_unsupported_frame_size`).
- [x] 4.3 All existing inter (zero-MV, sub-pel, residual, mvstack, SB-row) +
      general-intra fixtures byte-identical (no regression).
- [x] 4.4 Add `find_mv_stack` unit tests for the cross-SB-row availability.
- [x] 4.5 `cargo xtask ci` passes; `openspec validate --all` clean.

## 5. Deferred (out of scope, gated absent before output)
- [ ] 5.1 A partial (non-multiple-of-64) frame size needing edge clamping.
- [ ] 5.2 A multi-superblock skip == 0 residual (per-block transform sizes).
- [ ] 5.3 The deferred § 7.12.2 candidates inherited from
      `DECODE-INTER-MVSTACK-SPATIAL` (temporal, compound, warp, ref-MV bank,
      derived-SMVP, DRL reorder, scan-col wider reach, large-block MVP combos).
