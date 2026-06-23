# decode-general-intra-non64-multisb Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `decode-general-intra-non64-multisb`.

## Requirements
### Requirement: General intra multi-superblock (non-64x64) DC decode
The decoder SHALL decode single-tile 8-bit 4:2:0 intra key frames forming a
single superblock row — width a positive multiple of 64, height exactly 64 — on
the general intra path, iterating every superblock in the tile's MI range in
raster order per AV2 § 5.20.2.1 `decode_tile()` (`sbSize4 =
Num_4x4_Blocks_Wide[SbSize]` MI steps, with `clear_left_context()` at the start
of each superblock row), reusing one symbol decoder, the tile CDFs, the
§ 5.20.4.1 MI-size partition context, and the frame-spanning coefficient context
and reconstruction workspace so that later superblocks DC-predict their luma from
the already-reconstructed left neighbours. It SHALL size the reconstruction
workspace and decode limits to the real frame size (chroma half-resolution for
4:2:0). It SHALL reconstruct a full-superblock (64x64) block's § 7.13.2.13
`SMOOTH_PRED` chroma — resolved from the decoded `uv_mode` via § 5.20.5.3
`get_intra_uv_mode_set` for the non-directional luma subset — over § 7.13.2.1
`AboveRow` / `LeftCol` edges read from the partially-built frame, in addition to
DC chroma. For a full superblock `clear_block_decoded_flags` (§ 5.20.2) zeroes
the above-right region and the below-left is not yet decoded, so the § 7.13.2.13
top-right / bottom-left sentinels collapse to the edge-clamped last neighbour
sample the path supplies. It SHALL validate § 8.2.4 `exit_symbol()` after the
decoded superblocks. It SHALL keep the frozen `base_q_idx == 255` minimal hash
tier's strict 64x64 requirement (enforced in `validate_frame_core`) unchanged. It
SHALL reject — with a structured `decode/unsupported-feature` diagnostic — frames
whose height is not 64 or whose width is not a positive multiple of 64, SMOOTH
chroma on a sub-partitioned (non-full-superblock) block (whose § 7.13.2.1
above-right sentinel would need an already-decoded neighbour the path does not
read), other non-DC chroma modes (directional, PAETH, `SMOOTH_V`/`SMOOTH_H`),
multiple tiles, inter prediction, and in-loop filters, and SHALL NOT invoke AVM
or dav2d.

#### Scenario: Two-superblock 128x64 frame decodes to the oracle
- **WHEN** `splot decode` is given the committed multi-superblock intra key
  frame `syn-2sb-intra-128x64-q80.ivf`
- **THEN** the general intra path iterates both 64x64 superblocks, reconstructs
  the left superblock as flat DC luma 80 and the right superblock as flat DC
  luma 180 (DC-predicted from the reconstructed left neighbour), with flat
  chroma U=120 V=130 (the right superblock chroma coded as `SMOOTH_PRED`), and
  succeeds
- **AND** the reconstructed frame matches the avmdec and dav2d raw outputs
  byte-for-byte (md5 `88cf94a2d7b2a20c3212d96acdd456ef`)
- **AND** the decoded-frame hash is the pinned
  `18ba32ffb8d818689cbded3dbd5c44602bb091c1f9750c1bb062e6f80498540f`

#### Scenario: Existing 64x64 frames still decode bit-exact
- **WHEN** `splot decode` is given the committed 64x64 general intra fixtures
  (`syn-flat-intra-64x64-q80.ivf`, `syn-quad-intra-64x64-q80.ivf`,
  `syn-cos-intra-64x64-q180.ivf`, `syn-vsmooth-intra-64x64-q120.ivf`,
  `syn-hsmooth-intra-64x64-q120.ivf`)
- **THEN** each reconstructs to its previously pinned decoded-frame hash,
  unchanged by the multi-superblock generalization

#### Scenario: Unsupported sizes and chroma modes are rejected
- **WHEN** a frame height is not 64, a frame width is not a positive multiple of
  64, a block uses a non-DC chroma mode other than `SMOOTH_PRED`, or `SMOOTH_PRED`
  chroma appears on a sub-partitioned (non-full-superblock) block
- **THEN** the decoder returns a structured `decode/unsupported-feature`
  diagnostic without producing a frame
