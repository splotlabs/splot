## ADDED Requirements

### Requirement: 2-D-grid inter frame with cross-superblock-row MV prediction
The decoder SHALL decode a 2-D-grid single-reference inter frame whose geometry is
a full grid of 64x64 superblocks — width AND height each a positive multiple of 64
— iterated by the AV2 § 5.20.2.1 `decode_tile()` superblock raster loop, where a
block in a later superblock row predicts its motion vector from an already-decoded
superblock in an earlier row across the superblock-row boundary via the frame-wide
§ 7.11.2 mode context process and the § 7.12.2 Find MV stack process (spatial
single-prediction subset). The § 7.12.2.6 Scan point process SHALL invoke the
add-reference-motion-vector step only when `is_inside(mvRow, mvCol)` AND the
candidate location has been decoded (`RefFrames[mvRow][mvCol][0]` written for this
frame), so a probe into a not-yet-decoded superblock contributes no candidate. The
committed `syn-grid-inter-128x128-q80.ivf` SHALL be the verified target: a 128x128
frame that is a 2x2 grid of 64x64 superblocks, frame 0 an OBU_CLOSED_LOOP_KEY
DC_PRED intra key frame and frame 1 an OBU_REGULAR_TILE_GROUP inter frame whose
four superblocks are each a single 64x64 single-reference inter block, all skip=1 —
SB0 @ MI(0,0) NEWMV (a non-zero MV) and SB1 @ MI(0,16), SB2 @ MI(16,0), SB3 @
MI(16,16) NEARMV that reconstruct SB0's MV from the spatial-neighbour MV stack,
with SB2 and SB3 predicting across the superblock-row boundary.

The fixture SHALL be locally verified so that avmdec `--rawvideo --i420` and dav2d
`--demuxer ivf` decode the whole stream byte-for-byte identically (decoded-output
md5 `897bf67e72ec04cb7275fae08eab700c`, 49152 bytes), and SHALL be registered in
the conformance corpus validating clean with a reciprocal LOCAL-REFERENCE-EVIDENCE
entry. `splot decode --output-format raw` SHALL reproduce that raw output
byte-for-byte.

The decode SHALL be real: each block's § 5.20.7.6 mode_info, § 5.20.7.8 DRL, and
§ 5.20.7.20 read_mv symbols SHALL be read over the § 8.3.2 contexts derived from
the already-decoded neighbours (the § 5.20.7.2 `is_inter` / `skip_flag` contexts
and the § 7.11.2 NewMvContext), and the whole decode SHALL be guarded by § 8.2.4
`exit_symbol()` so that a wrong symbol read is rejected rather than emitting a
confident-but-wrong frame. The decoder SHALL NOT hardcode the motion vectors. The
existing single-superblock and single-superblock-row/column inter fixtures and all
general-intra fixtures SHALL continue to decode bit-exact.

The decoder SHALL reject, with a structured `decode/unsupported-feature`
diagnostic and no output, any frame outside the verified subset, including: a
partial frame size (a width or height that is not a multiple of 64), a
multi-superblock skip == 0 residual, and the deferred temporal / compound / warp /
ref-MV-bank / derived-SMVP / DRL-reorder MV candidates (once a block has a decoded
neighbour).

#### Scenario: 2-D-grid inter fixture decodes bit-exact to both oracles
- **WHEN** `splot decode --output-format raw` is given
  `syn-grid-inter-128x128-q80.ivf` with an output path
- **THEN** it exits 0 and writes 49152 bytes whose md5 is
  `897bf67e72ec04cb7275fae08eab700c`
- **AND** that output equals the avmdec `--rawvideo --i420` and dav2d
  `--demuxer ivf` raw output byte-for-byte

#### Scenario: A second-superblock-row block predicts a first-row motion vector
- **WHEN** the inter frame's four 64x64 superblocks are decoded in § 5.20.2.1
  raster order (sb_row outer, sb_col inner)
- **THEN** SB0 @ MI(0,0) is NEWMV with a non-zero motion vector and no decoded
  neighbour
- **AND** SB2 @ MI(16,0), in the second superblock row, reconstructs SB0's motion
  vector from the § 7.12.2 spatial-neighbour MV stack across the superblock-row
  boundary (not the zero global-MV fallback)

#### Scenario: A probe into a not-yet-decoded superblock yields no candidate
- **WHEN** the § 7.12.2 spatial scan of a superblock probes a motion-vector
  location belonging to a superblock that has not yet been decoded in raster order
- **THEN** the § 7.12.2.6 availability gate (`is_inside` && RefFrames-written)
  treats the unwritten cell as absent, so it contributes no candidate and the
  stack falls back to the zero global-MV candidate alone for a block with no
  decoded neighbour

#### Scenario: A partial (non-multiple-of-64) frame is rejected before any output
- **WHEN** `splot decode` is given an inter frame whose width or height is not a
  multiple of 64
- **THEN** it emits a structured `decode/unsupported-feature` diagnostic and
  writes no decoded output

#### Scenario: Existing inter and intra fixtures are unchanged
- **WHEN** `splot decode` is given the existing inter fixtures (zero-MV, sub-pel,
  residual, multi-block mvstack, single-superblock-row) and the general-intra
  fixtures
- **THEN** each decodes to its previously-recorded bit-exact output
