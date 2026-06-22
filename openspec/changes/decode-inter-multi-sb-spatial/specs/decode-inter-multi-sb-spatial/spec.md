## ADDED Requirements

### Requirement: Multi-superblock inter frame with cross-superblock MV prediction
The decoder SHALL decode a multi-superblock single-reference inter frame whose
geometry is a single superblock ROW (height 64, width a positive multiple of 64)
of 64x64 superblocks, iterated by the AV2 § 5.20.2.1 `decode_tile()` superblock raster
loop, where a block in a later superblock predicts its motion vector from the
immediately-prior superblock's reconstructed-edge neighbour across the superblock
boundary via the frame-wide § 7.11.2 mode context process and the § 7.12.2 Find MV
stack process (spatial single-prediction subset). The committed
`syn-2sb-inter-128x64-q80.ivf` SHALL be the verified target: a 128x64 frame of two
horizontally-adjacent 64x64 superblocks, frame 0 an OBU_CLOSED_LOOP_KEY DC_PRED
intra key frame and frame 1 an OBU_REGULAR_TILE_GROUP inter frame whose two
superblocks are each a single 64x64 single-reference inter block — SB0 NEWMV (a
non-zero MV) and SB1, in the second superblock, NEARMV that reconstructs SB0's MV
across the superblock boundary from the spatial-neighbour MV stack, both skip=1.

The fixture SHALL be locally verified so that avmdec `--rawvideo --i420` and dav2d
`--demuxer ivf` decode the whole stream byte-for-byte identically (decoded-output
md5 `477a993d671e93d37b92a0d368c238ff`, 24576 bytes), and SHALL be registered in
the conformance corpus validating clean with a reciprocal LOCAL-REFERENCE-EVIDENCE
entry. `splot decode --output-format raw` SHALL reproduce that raw output
byte-for-byte.

The decode SHALL be real: each block's § 5.20.7.6 mode_info, § 5.20.7.8 DRL, and
§ 5.20.7.20 read_mv symbols SHALL be read over the § 8.3.2 contexts derived from
the already-decoded neighbours (the § 5.20.7.2 `is_inter` / `skip_flag` contexts
and the § 7.11.2 NewMvContext), and the whole decode SHALL be guarded by § 8.2.4
`exit_symbol()` so that a wrong symbol read is rejected rather than emitting a
confident-but-wrong frame. The decoder SHALL NOT hardcode the motion vectors. The
existing single-superblock inter fixtures and all general-intra fixtures SHALL
continue to decode bit-exact.

The decoder SHALL reject, with a structured `decode/unsupported-feature`
diagnostic and no output, any frame outside the verified subset, including: a
single-superblock COLUMN (width 64, height greater than 64 — analytically correct
and locally verified, but deferred until its own committed 3-oracle fixture
lands), a full 2-D superblock grid (both dimensions greater than 64), a
multi-superblock skip == 0 residual, and the deferred temporal / compound / warp /
ref-MV-bank / derived-SMVP / DRL-reorder MV candidates.

#### Scenario: Multi-superblock inter fixture decodes bit-exact to both oracles
- **WHEN** `splot decode --output-format raw` is given
  `syn-2sb-inter-128x64-q80.ivf` with an output path
- **THEN** it exits 0 and writes 24576 bytes whose md5 is
  `477a993d671e93d37b92a0d368c238ff`
- **AND** that output equals the avmdec `--rawvideo --i420` and dav2d
  `--demuxer ivf` raw output byte-for-byte

#### Scenario: A second-superblock block predicts a first-superblock motion vector
- **WHEN** the inter frame's two 64x64 superblocks are decoded in § 5.20.2.1
  raster order
- **THEN** SB0 @ MI(0,0) is NEWMV with a non-zero motion vector and no decoded
  neighbour
- **AND** SB1 @ MI(0,16), in the second superblock, reconstructs SB0's motion
  vector from the § 7.12.2 spatial-neighbour MV stack across the superblock
  boundary (not the zero global-MV fallback)

#### Scenario: A full 2-D superblock grid is rejected before any output
- **WHEN** `splot decode` is given an inter frame whose width and height are both
  greater than 64
- **THEN** it emits a structured `decode/unsupported-feature` diagnostic and
  writes no decoded output

#### Scenario: Single-superblock inter and intra fixtures are unchanged
- **WHEN** `splot decode` is given the existing single-superblock inter fixtures
  (zero-MV, sub-pel, residual, multi-block mvstack) and the general-intra fixtures
- **THEN** each decodes to its previously-recorded bit-exact output
