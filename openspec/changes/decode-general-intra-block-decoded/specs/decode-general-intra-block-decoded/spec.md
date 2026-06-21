## ADDED Requirements

### Requirement: General intra per-block BlockDecoded SMOOTH_H split sub-block above-right decode
The decoder SHALL maintain the AV2 § 5.20.2.3 per-block `BlockDecoded` flag state
for the general intra path: a superblock-relative per-plane decoded-flag grid,
re-initialized at each § 5.20.2.1 superblock by § 5.20.2.3
`clear_block_decoded_flags` and updated after each decoded transform block. Using
this state via § 5.20.7.25 `count_top_right_avail`, the decoder SHALL decode a
SMOOTH_H_PRED luma SPLIT sub-block of size 32x32-or-larger (TX_SET_DCTONLY) whose
§ 7.13.2.1 above-right sentinel `AboveRow[w]` is the real reconstructed sample of
an already-decoded intra-superblock sibling, rather than the edge-clamped own
above-row sample. It SHALL validate § 8.2.4 `exit_symbol()` after the whole tile.
It SHALL still reject a SMOOTH_V luma SPLIT sub-block (which reads the below-left
sentinel `LeftCol[h]`) and a SMOOTH chroma SPLIT sub-block with a structured
`decode/unsupported-feature` diagnostic. It SHALL NOT change DC block decode, and
SHALL NOT handle directional sub-blocks, non-DCTONLY-size non-DC sub-blocks,
multiple tiles, inter prediction, in-loop filters, or invoke AVM or dav2d.

#### Scenario: SMOOTH_H split sub-block reads the decoded above-right sibling
- **WHEN** `splot decode` is given the committed partition-tree intra key frame
  `syn-shsplit-intra-64x64-q80.ivf`, whose 64x64 superblock SPLITs into four
  32x32 squares with a SMOOTH_H_PRED bottom-left sub-block
- **THEN** the general intra path decodes and reconstructs each 32x32 leaf in
  § 5.20.3.1 decode order and succeeds
- **AND** the bottom-left 32x32 SMOOTH_H block's § 7.13.2.1 above-right sentinel
  `AboveRow[w]` is the real reconstructed bottom-left corner (210) of the
  already-decoded top-right 32x32 sibling (`count_top_right_avail` = 8), so its
  reconstructed right column is ~211, not the ~51 the edge clamp would produce
- **AND** the output equals the avmdec `--rawvideo --i420` and dav2d
  `--demuxer ivf` raw outputs byte-for-byte (raw md5
  `88ea298073104752646aab5f718fdc31`)
- **AND** the decoded-frame hash is the pinned
  `296f15949d88b26b5797bffdb15c6c36dc46bf6976bad59f7995e2443e1b418a`

#### Scenario: SMOOTH chroma split sub-block still rejects
- **WHEN** `splot decode` is given the committed negative companion
  `syn-svsplit-intra-64x64-q140.ivf`, a SPLIT 64x64 superblock the encoder codes
  with a SMOOTH chroma sub-block (the stream validates clean and avmdec / dav2d
  agree)
- **THEN** the general intra decoder rejects it with a structured
  `decode/unsupported-feature` diagnostic whose reason is
  `general_intra_smooth_chroma_subblock`, rather than producing wrong output

#### Scenario: Existing general intra fixtures are unchanged
- **WHEN** `splot decode` is given the existing general intra fixtures
  (`syn-flat-intra-64x64-q80.ivf`, `syn-quad-intra-64x64-q80.ivf`,
  `syn-deep-intra-64x64-q120.ivf`, `syn-grid-intra-128x128-q80.ivf`, and the
  remaining committed vectors)
- **THEN** they decode bit-exactly to their pinned decoded-frame hashes
