## ADDED Requirements

### Requirement: distinct-neighbour-MV § 7.12.2 stack ordering pinned bit-exact
The decoder SHALL decode a distinct-neighbour-MV single-reference inter frame whose
inter superblock is an AV2 § 5.20.3 SPLIT into four 32x32 leaves, each carrying a
DIFFERENT motion vector, such that the per-neighbour § 7.12.2.6 scan-point ORDERING
(left-before-above precedence) and the § 5.20.7.8 DRL slot selection are pinned
bit-exact. The committed `syn-2frame-inter-mvorder-64x64.ivf` SHALL be the verified
target: a 64x64 frame, frame 0 an OBU_CLOSED_LOOP_KEY DC_PRED intra key frame (four
flat 32x32 quadrants) and frame 1 an OBU_REGULAR_TILE_GROUP inter frame whose 64x64
superblock is SPLIT into four 32x32 single-reference inter blocks, all skip=1, each
carrying a distinct motion vector — block 0 @ MI(0,0) NEWMV col 64, block 1 @
MI(0,8) NEWMV col -32, block 2 @ MI(8,0) NEWMV col 32, and the interior block 3 @
MI(8,8) NEARMV with RefMvIdx 1 over a § 7.12.2 MV stack whose slot 0 is the LEFT
neighbour (block 2, col 32, from the step-7 `scan_point(bh4 - 1, -1)` probe) and
slot 1 is the ABOVE neighbour (block 1, col -32, from the step-8
`scan_point(-1, bw4 - 1)` probe), so RefMvIdx 1 reconstructs col -32 (the above
neighbour) directly.

The fixture SHALL be locally verified so that avmdec `--rawvideo --i420` and dav2d
`--demuxer ivf` decode the whole stream byte-for-byte identically (decoded-output
md5 `284e1450b42180f02de7415ab0367bfe`, 12288 bytes), and SHALL be registered in
the conformance corpus validating clean with a reciprocal LOCAL-REFERENCE-EVIDENCE
entry. `splot decode --output-format raw` SHALL reproduce that raw output
byte-for-byte.

The decode SHALL be real: each block's § 5.20.7.6 mode_info, § 5.20.7.8 DRL, and
§ 5.20.7.20 read_mv symbols SHALL be read over the § 8.3.2 contexts derived from
the already-decoded neighbours, and the whole decode SHALL be guarded by § 8.2.4
`exit_symbol()` so that a wrong symbol read is rejected rather than emitting a
confident-but-wrong frame. The decoder SHALL NOT hardcode the motion vectors. The
existing single-block, multi-block, multi-superblock, and 2-D-grid inter fixtures
and all general-intra fixtures SHALL continue to decode bit-exact.

The committed fixture proves the per-neighbour spatial scan-point ORDERING
(left-before-above precedence with DISTINCT neighbour MVs) and the § 5.20.7.8 DRL
slot selection: the interior block 3 reconstructs the slot-1 (above) candidate, so
a reversed (above-before-left) order would reconstruct the slot-0 (left, col 32)
candidate and mismatch both oracles. Because every leaf is exactly 32x32
(`Block_Width` / `Block_Height` == 32, NOT > 32), the § 7.12.2.20 large-block
(> 32x32) extra MVP combinations are inapplicable, so the fixture pins the ordering
without that step; § 7.12.2.20 remains deferred for the > 32x32 leaves it does not
yet model and stays guarded by the § 5.20.7.8 `inter_block_drl_idx_out_of_range`
reject. The decoder SHALL continue to reject, with a structured
`decode/unsupported-feature` diagnostic and no output, any frame outside the
verified subset, including the deferred temporal / compound / warp / ref-MV-bank /
derived-SMVP / DRL-reorder MV candidates and a multi-superblock skip == 0 residual.

#### Scenario: distinct-MV inter fixture decodes bit-exact to both oracles
- **WHEN** `splot decode --output-format raw` is given
  `syn-2frame-inter-mvorder-64x64.ivf`
- **THEN** it reproduces the avmdec / dav2d raw output byte-for-byte (md5
  `284e1450b42180f02de7415ab0367bfe`, 12288 bytes)
- **AND** block 0 @ MI(0,0) is NEWMV col 64, block 1 @ MI(0,8) is NEWMV col -32,
  block 2 @ MI(8,0) is NEWMV col 32, each a DISTINCT motion vector
- **AND** the interior block 3 @ MI(8,8) is NEARMV RefMvIdx 1 reconstructing
  col -32 (the above neighbour), not col 32 (the left neighbour)

#### Scenario: the § 7.12.2 stack orders the left neighbour before the above neighbour
- **WHEN** a 32x32 block has a decoded LEFT neighbour and a decoded ABOVE
  neighbour carrying DIFFERENT motion vectors
- **THEN** the § 7.12.2.6 ordered spatial scan places the LEFT neighbour's motion
  vector at stack slot 0 and the ABOVE neighbour's at stack slot 1
- **AND** a NEARMV block with RefMvIdx 1 reconstructs the ABOVE neighbour's motion
  vector

#### Scenario: the § 7.12.2.20 large-block step stays inapplicable to 32x32 leaves
- **WHEN** the verified fixture's 32x32 inter leaves build their § 7.12.2 MV stack
- **THEN** the § 7.12.2.20 large-block (`Block_Width` > 32 AND `Block_Height` > 32)
  extra MVP combinations are not invoked, because the leaves are exactly 32x32
- **AND** the spatial scan-point ordering is pinned without implementing
  § 7.12.2.20

#### Scenario: existing inter and intra fixtures are unchanged
- **WHEN** `splot decode` is given the existing inter fixtures (zero-MV, sub-pel,
  residual, multi-block mvstack, single-superblock-row, 2-D-grid) and the
  general-intra fixtures
- **THEN** each decodes to its previously-recorded bit-exact output
