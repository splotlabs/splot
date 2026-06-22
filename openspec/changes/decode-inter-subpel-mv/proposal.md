## Why

The first inter frame decodes bit-exact (`DECODE-FIRST-INTER-FRAME-FRONTIER`),
but only the zero-MV skip case, where AV2 § 7.13.3.18 motion compensation reduces
to a straight reference-sample copy. Real inter content needs sub-pel motion
compensation: a non-zero, fractional motion vector that samples the reference
through the § 7.13.3.18 separable interpolation-filter convolution. The
convolution kernel itself is merged (`RECON-SUBPEL-MC`,
`splot_recon::subpel_predict_block`); this change wires it into the decode path
and reads the motion vector and interpolation filter from the bitstream.

The smallest bit-exact-verifiable sub-pel step is a two-frame stream (1 intra key
+ 1 inter frame) whose inter frame is a single 64x64 block, single reference,
NEWMV with a fractional (EighthPel) motion vector, a SWITCHABLE interpolation
filter, and skip=1 (no residual). The reconstructed frame is a fractionally
shifted version of the key frame — distinct from the zero-MV copy.

## What Changes

- Add Feature ID `DECODE-INTER-SUBPEL-MV`.
- Add the project-owned `syn-2frame-subpel-inter-64x64.ivf` fixture (frame 0 =
  a general-intra DC_PRED half-cosine key frame; frame 1 = an
  OBU_REGULAR_TILE_GROUP single-reference NEWMV inter frame with an EighthPel
  `(0, -4)` horizontal half-sample sub-pel motion vector, a SWITCHABLE
  `EIGHTTAP_SHARP` interpolation filter, and skip=1). Prove avmdec `--rawvideo
  --i420` and dav2d `--demuxer ivf` decode the whole stream byte-for-byte
  identically (decoded-output md5 `a0e82de3a95bb4b519c4c84ffa2ba816`, 12288
  bytes).
- Implement the AV2 § 5.20.7.20 SHELL-coded `read_mv()` (shell_set, the EighthPel
  shell_class, joint_shell_last_two_classes, shell_offset_low_class / class2 /
  other_class, col_mv_greater, the § 4.11.13 NS(n) col_remainder, col_mv_index)
  plus the § 5.20.7.13 explicit `mv_sign` sign pass, over the zero no-neighbour
  predictor.
- Implement the AV2 § 7.13.3.17 motion-vector scaling (startX/startY/stepX/stepY)
  and the § 7.13.3.18 reference-clipping bounds (firstX/firstY/lastX/lastY) per
  plane (luma + 4:2:0 chroma), and feed them with a packed `ReferencePlaneView`
  to `splot_recon::subpel_predict_block` for the motion-compensated prediction.
- Read the § 5.20.7.6 `interp_filter` SWITCHABLE symbol (when the frame filter is
  SWITCHABLE and `needs_interp_filter()` is 1) and use it for the convolution.
- Add the AV2 § 9.3 SHELL-coded MV CDF banks and the interp_filter CDF to the
  tile/block CDF subset, selected per § 8.3.2.
- Relax `validate_inter_frame_core` / the inter block decode to admit a
  single-reference NEWMV sub-pel skip=1 block with a SWITCHABLE-or-fixed
  interpolation filter, keeping every assumed-absent header/mode fact rejected
  (residual skip=0, compound, multi-reference, motion modes, OBMC, warp,
  flex-mvres, adaptive-mvd, bawp, cwp) so § 8.2.4 `exit_symbol()` can never be
  bypassed.
- Register the fixture in the conformance manifest (`expect = "clean"`) and add
  the reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## Capabilities

### New Capabilities
- `decode-inter-subpel-mv`: A committed, oracle-verified minimal sub-pel inter
  decode target (1 intra key + 1 NEWMV sub-pel skip inter frame) decoded
  bit-exact via the SHELL-coded `read_mv`, the § 7.13.3.17 MV scaling, and the
  § 7.13.3.18 interpolation-filter convolution.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the sub-pel
  inter decode.

## Impact

- Adds `tests/conformance/vectors/valid/syn-2frame-subpel-inter-64x64.ivf` and
  decode tests in `crates/splot-decode/src/runtime_minimal/inter/tests.rs`.
- Adds `crates/splot-decode/src/runtime_minimal/inter/read_mv.rs` and
  `crates/splot-decode/src/runtime_minimal/inter/mv_scaling.rs`; updates
  `crates/splot-decode/src/runtime_minimal/inter/{mc,block}.rs`,
  `crates/splot-decode/src/runtime_minimal/inter.rs`, and the tile/block CDF
  subset (`crates/splot-decode/src/tile_payload/cdf{,/block_rows}.rs`).
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and
  the generated status/coverage docs.
- No public API, dependency graph, encoder, or validator changes. Inter residual
  (skip=0), compound / multi-reference prediction, motion modes (OBMC / warp),
  non-64x64 / multi-block inter, in-loop filters, and live AVM/dav2d invocation
  in CI remain out of scope.
