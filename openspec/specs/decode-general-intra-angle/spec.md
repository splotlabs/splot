# decode-general-intra-angle Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `decode-general-intra-angle`.

## Requirements
### Requirement: General intra single-block directional-angle luma decode
The decoder SHALL reconstruct the § 7.13.2.8 `D135_PRED` (pAngle 135) directional
luma prediction mode for the top-left (no-neighbour) 64x64 superblock block of a
64x64 8-bit 4:2:0 intra key frame on the general intra path. It SHALL reconstruct
the § 5.20.5.3 `y_mode_offset` escape (`y_mode_set == 0`,
`y_mode_index == MODE_INDEX_COUNT - 1`): it SHALL read `y_mode_offset` through the
`TileYModeOffsetCdf` selector (sharing the `y_mode_index` § 8.3.2 context, with
defaults from `Default_Y_Mode_Offset_Cdf`) and resolve the typed `YMode` and
`AngleDeltaY` via `get_intra_y_mode_set` (over `Default_Mode_List_Y` for the
top-left no-directional-neighbour case) and the § 5.20.5.3 directional reorder
(`Reordered_Y_Mode`, `TOTAL_ANGLE_DELTA_COUNT`, `MAX_ANGLE_DELTA`). It SHALL build
the § 7.13.2.8 prediction over the § 7.13.2.1 no-neighbour fallback edges (8-bit:
`AboveRow` samples `(1 << (BitDepth - 1)) - 1`, `LeftCol` samples
`(1 << (BitDepth - 1)) + 1`, the shared corner `1 << (BitDepth - 1)`) using the
shared `splot-recon` middle-angle directional predictor, and SHALL add the
§ 5.20.7.27 residual over that per-sample prediction. It SHALL validate § 8.2.4
`exit_symbol()` after the coefficients. It SHALL gate the directional block decode
to pAngle 135 with `AngleDeltaY == 0` at the top-left (no-neighbour) 64x64
superblock — the § 5.20.8.2 `get_tx_set` `TX_SET_DCTONLY` case where the transform
is forced `DCT_DCT` with no `intra_tx_type` signaled and the
`enable_intra_edge_filter` / IDIF / upsample edge synthesis is a no-op — with DC
chroma, rejecting non-zero angle deltas, the other directional modes, sub-64x64
directional blocks (which can signal a mode-dependent transform type),
neighbour-having directional blocks, and non-DC chroma with a structured
`decode/unsupported-feature` diagnostic before any reconstruction. It SHALL NOT
handle multi-block directional prediction, non-64x64 frames, inter prediction,
in-loop filters, or invoke AVM or dav2d.

#### Scenario: Directional D135 frame decodes to the oracle
- **WHEN** `splot decode` is given the committed single-block intra key frame
  `syn-hedge-intra-64x64-q80.ivf`
- **THEN** the general intra path reconstructs the § 5.20.5.3 `y_mode_offset`
  escape to `D135_PRED` (`AngleDeltaY == 0`), builds the § 7.13.2.8 prediction
  over the no-neighbour fallback edges, adds the § 5.20.7.27 residual, with DC
  chroma, and succeeds
- **AND** the reconstructed luma is a genuinely non-flat reconstruction (top half
  near 40, bottom half near 210) matching the avmdec and dav2d raw outputs (md5
  `1179bcc873c1d1ac49c2c032f11ca44d`)
- **AND** the decoded-frame hash is the pinned
  `b15f267ec6e99ca4d96a70f38bffe5f798ee4c33ad3aaec23761a1ea74b0be33`

#### Scenario: Unsupported directional cases are rejected before reconstruction
- **WHEN** a block uses a directional luma mode other than `D135_PRED`, a
  directional luma mode with a non-zero `AngleDeltaY`, a directional luma mode on a
  block smaller than the 64x64 superblock or with an above/left neighbour, or a
  non-DC chroma mode
- **THEN** the decoder returns a structured `decode/unsupported-feature`
  diagnostic without reconstructing the block
