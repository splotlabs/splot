# Proposal: segmentation and tile-params foundation

## Summary

Implement the reusable AV2 descriptor, segmentation, and tile-partitioning
foundations that remove the three bounded holes the sequence-header and
multi-frame-header parsers currently stop at: `seg_info()` (§5.4.9) and the
sequence `tile_params()` helper (§5.18.7.3). With these in place the sequence
header and multi-frame header parse fully (for valid level/tier streams) and the
validator runs the §5.2.1 payload-tail check on them instead of bounding early.

## Why

`parse-sequence-header` and `frame-activation-hls-skeleton` intentionally left
three bounded holes:

- `sequence_segment_config()` stopped after `seq_allow_seg_info_change`, before
  `seg_info(MaxSegments)`.
- `sequence_tile_config()` stopped after `allow_tile_info_change`, before
  `tile_params(...)`.
- `multi_frame_header_obu()` stopped after `mfh_allow_seg_info_change`, before
  `seg_info(...)`.

All three need the same shared primitives: the AV2 `su(n)` signed descriptor
(§4.11.7), the reusable `seg_info(numSegments)` parser (§5.4.9) with its exact
segmentation-feature tables, and the tile-partitioning helpers (`tile_log2`,
`uniform_spacing`, the block-size and level/tier scaling tables) used by
`tile_params()` (§5.18.7.3–§5.18.7.7). These are also prerequisites for the future
full frame-header `segmentation_params()` and `tile_info()`.

Closing the holes lets the validator promote sequence headers and multi-frame
headers with segment/tile info to full payload-tail validation, catching truncated
or malformed payloads that the bounded parse used to miss.

## What changes

- Add `BitReader::read_su(width)` for the AV2 `su(n)` descriptor (§4.11.7).
- Add a reusable `parse_seg_info(reader, numSegments)` parser with the exact
  `Segmentation_Feature_Bits` / `Segmentation_Feature_Signed` /
  `Segmentation_Feature_Max` tables (§5.4.9).
- Wire `seg_info()` into `sequence_segment_config()` and `multi_frame_header_obu()`,
  removing their `seg_info()` bounded holes.
- Add the tile-partitioning helpers (`tile_log2`, `uniform_spacing`, the block-size
  conversion tables, and the level/tier `Tile_Width_Scaling_Factor` /
  `Tile_Area_Scaling_Factor` tables) and implement `tile_params()` for the sequence
  tile config (§5.18.7.3 with §5.18.7.5/§5.18.7.7 helpers).
- Promote the validator: fully parsed sequence headers and multi-frame headers run
  the §5.2.1 payload-tail check; MFH availability is gated on it.
- Add local tile semantic diagnostics (`tile-params/*`) and the `tile-params/`
  diagnostic namespace.
- Update the implementation matrix, diagnostics registry, generated feature status,
  current-validator-state, and tests.

## Non-goals

- Full §5.18 frame header, frame `segmentation_params()`, or frame `tile_info()` as a
  live parser path.
- `frame_header_copy()`, tile-group payload, or entropy-coded tile data.
- LCR / OPS / atlas / metadata / QM / film grain / buffer-removal OBUs.
- Annex A level/tier conformance beyond the constants `tile_params()` needs.
- Encoder or bitstream-writer work.

## Feature IDs

- `AV2-4.11.7-SU`
- `AV2-5.4.9-SEGMENT-INFO`
- `AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG`
- `AV2-5.4.2-SEQUENCE-TILE-CONFIG`
- `AV2-5.7-MULTI-FRAME-HEADER`
- `AV2-5.18.7.3-TILE-PARAMS`
- `AV2-5.18.7-SEGMENTATION-TILING` (umbrella; remains partial)

## Acceptance criteria

- `BitReader::read_su(n)` decodes the §4.11.7 sign-extension exactly, with positive,
  negative, EOF, and invalid-width coverage, and never panics.
- `parse_seg_info()` parses the 8- and 16-segment paths, reads the signed quantizer
  feature via `su(10)`, clips per the feature tables, and zero-initializes unused
  slots.
- A sequence header with `seq_seg_info_present_flag = 1` parses fully and is
  payload-tail validated; a malformed tail after the segment info is now diagnosed.
- A multi-frame header with `mfh_seg_info_present_flag = 1` parses fully (no longer
  bounded) and is payload-tail validated and recorded as available.
- A sequence header with `seq_tile_info_present_flag = 1` (uniform or non-uniform)
  parses fully for valid level/tier streams, with no bounded tile-params status.
- The validator emits `tile-params/tile-cols-out-of-range`,
  `tile-params/tile-rows-out-of-range`,
  `tile-params/nonuniform-cols-do-not-cover-frame`, and
  `tile-params/nonuniform-rows-do-not-cover-frame` as documented.
- Frame `tile_info()`, `frame_header()`, and tile-group payload rows remain
  partial/todo; `AV2-5.18.7-SEGMENTATION-TILING` is not marked done.
- `cargo xtask ci` and
  `openspec validate segmentation-tile-params-foundation --strict` pass.
