# decode-general-intra-multirow Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `decode-general-intra-multirow`.

## Requirements
### Requirement: General intra single-row-or-column multi-superblock decode
The decoder SHALL decode single-tile 8-bit 4:2:0 intra key frames whose width and
height are positive multiples of 64 and which form a single ROW (height == 64) or
single COLUMN (width == 64) of 64x64 superblocks, on the general intra path,
iterating every superblock in the tile's MI range in raster order per AV2
§ 5.20.2.1 `decode_tile()` with `clear_left_context()` at the start of each
superblock row, so that a second-row superblock DC-predicts its luma from the
already-reconstructed first-row above neighbour. The single-row-or-column
restriction SHALL keep every full-superblock § 7.13.2.13 `SMOOTH_PRED` chroma
block free of a decoded above-right neighbour (a row-0 block has no above; a
rightmost-column block has no above-right), so the § 7.13.2.1 `AboveRow[w]`
sentinel the path builds by edge-clamping equals the spec value. It SHALL keep the
frozen `base_q_idx == 255` minimal hash tier's strict 64x64 requirement unchanged.
It SHALL reject — with a structured `decode/unsupported-feature` diagnostic — a
2-D grid frame (both width and height greater than 64, whose non-rightmost row>0
superblock has a decoded above-right the SMOOTH chroma sentinel does not read), a
frame whose width or height is not a positive multiple of 64, a directional luma
mode, SMOOTH chroma on a sub-partitioned (non-full-superblock) block, other
non-DC chroma modes, multiple tiles, inter prediction, and in-loop filters, and
SHALL NOT invoke AVM or dav2d.

#### Scenario: Single-column 64x128 frame decodes to the oracle
- **WHEN** `splot decode` is given the committed single-column multi-superblock
  intra key frame `syn-2sbcol-intra-64x128-q80.ivf`
- **THEN** the general intra path iterates the two vertically stacked 64x64
  superblocks, with the second-row superblock DC-predicting its luma from the
  reconstructed first-row neighbour, and succeeds
- **AND** the top superblock luma is flat 80, the bottom superblock luma is flat
  180, and chroma is U=120 / V=130, matching the avmdec and dav2d raw outputs
  byte-for-byte (md5 `bd09ea820e68abdc002e80c8c4e30bb7`)
- **AND** the decoded-frame hash is the pinned
  `3ee739a805e13597ff7d75659dd1e0150113bf4782c4d69e1d27ae942d6c10a0`

#### Scenario: 2-D grid frames are rejected
- **WHEN** `splot decode` is given a frame whose width and height are both greater
  than 64 (a 2-D grid of superblocks)
- **THEN** the decoder returns a structured `decode/unsupported-feature`
  diagnostic without producing a frame (the above-right SMOOTH chroma sentinel is
  not yet read from the reconstructed neighbour)

#### Scenario: Existing single-row frames still decode bit-exact
- **WHEN** `splot decode` is given the committed 64x64 and 128x64 general intra
  fixtures
- **THEN** each reconstructs to its previously pinned decoded-frame hash,
  unchanged by the single-column generalization
