## ADDED Requirements

### Requirement: Multi-block inter frame with neighbour-predicted motion vectors
The decoder SHALL decode a multi-block single-reference inter frame whose 64x64
superblock is AV2 § 5.20.3 split into multiple inter blocks, where a later block's
motion vector is predicted from a decoded neighbour block via the § 7.11.2 mode
context process and the § 7.12.2 Find MV stack process (spatial single-prediction
subset). The committed `syn-2frame-inter-mvstack-64x64.ivf` SHALL be the verified
target: frame 0 is an OBU_CLOSED_LOOP_KEY DC_PRED intra key frame and frame 1 is
an OBU_REGULAR_TILE_GROUP inter frame whose 64x64 superblock is split into four
32x32 single-reference inter blocks — block 0 NEWMV (a non-zero MV) and the later
three NEARMV that reconstruct block 0's MV from the spatial-neighbour MV stack,
all skip=1.

The fixture SHALL be locally verified so that avmdec `--rawvideo --i420` and dav2d
`--demuxer ivf` decode the whole stream byte-for-byte identically (decoded-output
md5 `e5b581a55433785c0071b635d5642083`, 12288 bytes), and SHALL be registered in
the conformance corpus validating clean with a reciprocal LOCAL-REFERENCE-EVIDENCE
entry. `splot decode --output-format raw` SHALL reproduce that raw output
byte-for-byte.

The decode SHALL be real: each block's § 5.20.7.6 mode_info, § 5.20.7.8 DRL, and
§ 5.20.7.20 read_mv symbols SHALL be read over the § 8.3.2 contexts derived from
the already-decoded neighbours (the § 5.20.7.2 `is_inter` / `skip_flag` contexts
and the § 7.11.2 NewMvContext), and the whole decode SHALL be guarded by § 8.2.4
`exit_symbol()` so that a wrong symbol read is rejected rather than emitting a
confident-but-wrong frame. The decoder SHALL NOT hardcode the motion vectors. The
existing single-block inter fixtures and all general-intra fixtures SHALL continue
to decode bit-exact.

The decoder SHALL reject, with a structured `decode/unsupported-feature`
diagnostic and no output, any frame outside the verified spatial single-prediction
subset, including: temporal MV candidates (`use_ref_frame_mvs`), compound
prediction, warp candidates, the reference MV bank (`enable_refmvbank`), the DRL
reorder sort (`enable_drl_reorder`), global (warp) motion, and a multi-block
skip == 0 residual.

#### Scenario: Multi-block inter fixture decodes bit-exact to both oracles
- **WHEN** `splot decode --output-format raw` is given
  `syn-2frame-inter-mvstack-64x64.ivf` with an output path
- **THEN** it exits 0 and writes 12288 bytes whose md5 is
  `e5b581a55433785c0071b635d5642083`
- **AND** that output equals the avmdec `--rawvideo --i420` and dav2d
  `--demuxer ivf` raw output byte-for-byte

#### Scenario: A later block predicts an earlier block's motion vector
- **WHEN** the inter frame's four 32x32 blocks are decoded in § 5.20.3 order
- **THEN** block 0 @ MI(0,0) is NEWMV with a non-zero motion vector
- **AND** each later NEARMV block reconstructs block 0's motion vector from the
  § 7.12.2 spatial-neighbour MV stack (not the zero global-MV fallback)

#### Scenario: Single-block inter and intra fixtures are unchanged
- **WHEN** `splot decode` is given the existing single-block inter fixtures
  (zero-MV, sub-pel, residual) and the general-intra fixtures
- **THEN** each decodes to its previously-recorded bit-exact output
