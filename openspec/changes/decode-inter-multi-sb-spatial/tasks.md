# Tasks

## 1. Fixture (verify oracles first)
- [x] 1.1 Generate `syn-2sb-inter-128x64-q80.ivf` from a project-owned synthetic
      Y4M (128x64: two flat 64x64 luma superblocks left-100 right-150 + flat
      chroma; frame 1 shifted left 6 luma samples) at `--qp 80 --sb-size 64
      --min-partition-size 64 --max-partition-size 64` with broad decode tools
      disabled.
- [x] 1.2 Confirm avmdec `--rawvideo --i420` == dav2d `--demuxer ivf`
      byte-for-byte (md5 `477a993d671e93d37b92a0d368c238ff`, 24576 bytes).
- [x] 1.3 Confirm via splot instrumentation that the inter frame is two 64x64
      superblocks: SB0 @ MI(0,0) NEWMV (col 48, has_neighbour=false), SB1 @
      MI(0,16) NEARMV (col 48, NewMvContext 3, has_neighbour=true — predicted from
      SB0 across the superblock boundary, not the zero fallback).
- [x] 1.4 Register in the conformance manifest + reciprocal
      LOCAL-REFERENCE-EVIDENCE entry.

## 2. Lift the frame-size gate (the only decoder change)
- [x] 2.1 Lift the § 5.18.3 `validate_inter_frame_core` frame-size gate from
      exactly 64x64 to a single superblock ROW (height 64, width a multiple of 64)
      of 64x64 superblocks.
- [x] 2.2 Reject a full 2-D superblock grid (both dimensions > 64) AND the
      single-superblock column (width 64, height > 64; analytically correct +
      locally verified but deferred until its own committed fixture) with a
      structured `decode/unsupported-feature` diagnostic.
- [x] 2.3 Confirm the § 5.20.2.1 SB raster loop, the frame-wide find_mv_stack
      grid, `decode_inter_blocks`, and the tile-payload boundary derivation need
      no change (geometry-agnostic).

## 3. Verify + gate
- [x] 3.1 `splot decode syn-2sb-inter-128x64-q80.ivf --output-format raw` ==
      oracle md5 byte-for-byte.
- [x] 3.2 Prove the OLD code rejected the 128x64 fixture
      (`inter_unsupported_frame_size`).
- [x] 3.3 All existing inter (zero-MV, sub-pel, residual, mvstack) + general-intra
      fixtures byte-identical (no regression).
- [x] 3.4 `cargo xtask ci` passes; `openspec validate --all` clean.

## 4. Deferred (out of scope, gated absent before output)
- [ ] 4.1 A full 2-D superblock grid (both dimensions > 64): a non-leftmost
      non-top superblock's § 7.12.2 above-right / below-left scan positions reach a
      not-yet-decoded superblock.
- [ ] 4.2 A multi-superblock skip == 0 residual (per-block transform sizes).
- [ ] 4.3 The deferred § 7.12.2 candidates inherited from
      `DECODE-INTER-MVSTACK-SPATIAL` (temporal, compound, warp, ref-MV bank,
      derived-SMVP, DRL reorder, scan-col wider reach, large-block MVP combos).
