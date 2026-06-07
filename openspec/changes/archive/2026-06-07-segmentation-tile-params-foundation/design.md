# Design: segmentation and tile-params foundation

## 1. Boundary

This change adds three reusable AV2 primitives and wires two of them into existing
parsers. It does **not** start the full frame header. The new code is:

- `su(n)` descriptor (`splot-core/src/bitio.rs`);
- `seg_info(numSegments)` (`splot-core/src/segment.rs`);
- the tile-partitioning helpers and `tile_params()` (`splot-core/src/tile.rs`).

`frame_header()` and `tile_group_obu()` stay prefix-only. Frame
`segmentation_params()` and frame `tile_info()` are out of scope; only the sequence
`tile_params()` call site is wired.

## 2. `su(n)` (§4.11.7)

`read_su(width)` reads `width` bits MSB-first with the existing fixed-width reader,
then sign-extends:

```text
value = f(width)
signMask = 1 << (width - 1)
if (value & signMask) value = value - 2 * signMask
return value
```

`width` is bounded to `1..=32`; `width == 0` and `width > 32` return a structured
`Error` (no panic). The result is `i32` (the widest signed value `seg_info()` needs
is `su(10)`).

## 3. `seg_info(numSegments)` (§5.4.9)

Reusable `parse_seg_info(reader, num_segments)` over `SEG_LVL_MAX = 3` features and
up to `MAX_SEGMENTS = 16` segments. The exact §5.4.9 constant tables are:

```text
Segmentation_Feature_Bits[SEG_LVL_MAX]   = { 9, 0, 0 }
Segmentation_Feature_Signed[SEG_LVL_MAX] = { 1, 0, 0 }
Segmentation_Feature_Max[SEG_LVL_MAX]    = { MAXQ_BITS, 0, 0 }   // MAXQ_BITS = 255 + 4*24 = 351
```

Per `(i, j)`: read `feature_enabled` `f(1)`; if enabled, read the value with
`bitsToRead = Segmentation_Feature_Bits[j]` — for the signed feature (`j == 0`),
`su(1 + bitsToRead)` clipped to `[-limit, limit]`; otherwise `f(bitsToRead)` clipped
to `[0, limit]` (for `j == 1, 2`, `bitsToRead == 0` so no value bits are read). Unused
slots default to disabled / data 0.

`SegmentInfo` stores `num_segments` and a `[[SegmentFeature; SEG_LVL_MAX]; MAX_SEGMENTS]`
matrix of `{ enabled, data }`, exposing enough for future inspector output.

Sequence wiring: when `seq_seg_info_present_flag`, read `seq_allow_seg_info_change`,
compute `MaxSegments = enable_ext_seg ? 16 : 8`, then `parse_seg_info(MaxSegments)`.

MFH wiring: when `mfh_seg_info_present_flag`, read `mfh_ext_seg_flag` and
`mfh_allow_seg_info_change`, then `parse_seg_info(mfh_ext_seg_flag ? 16 : 8)`. The old
`unimplemented_at = Some("AV2-5.4.9-SEGMENT-INFO")` bound is removed.

## 4. `tile_params()` (§5.18.7.3)

`parse_tile_params(reader, input)` implements §5.18.7.3 with the §5.18.7.5
`uniform_spacing` and §5.18.7.7 `tile_log2` helpers. `TileParamsInput` carries
`frame_width`, `frame_height`, `uniform_sb_size`, `sb_size`, `is_bridge`, `seq_tier`,
and `seq_level_idx`.

Exact constants from the spec conversion tables:

```text
// §9.3 conversion tables, for the three sequence superblock sizes
Num_4x4_Blocks_Wide: BLOCK_64X64 = 16, BLOCK_128X128 = 32, BLOCK_256X256 = 64
Mi_Width_Log2:       BLOCK_64X64 = 4,  BLOCK_128X128 = 5,  BLOCK_256X256 = 6
MAX_TILE_COLS = 64, MAX_TILE_ROWS = 64, MAX_TILE_WIDTH = 4096, MAX_TILE_AREA = 4096*2304
Tile_Width_Scaling_Factor[2][31], Tile_Area_Scaling_Factor[2][31]   // §A scaling tables
```

The scaling-table lookup returns `Option<u32>` (reserved level indices → `None`).
For `seq_level_idx == 31` the spec's unconstrained fallback applies
(`maxTileWidthSb = sbCols`, `maxTileAreaSb = sbCols * sbRows`). A reserved level has no
defined bit layout, so `parse_tile_params` returns `Error::Unimplemented` for it; the
sequence tile config converts that to `params: None` (a documented bounded residual),
which keeps `SequenceTileConfig::unimplemented_at()` returning
`AV2-5.4.2-SEQUENCE-TILE-CONFIG` only for reserved (non-conformant) levels — never for
valid streams.

`TileParams` records `tile_cols`, `tile_rows`, `tile_cols_log2`, `tile_rows_log2`,
`sb_cols`, `sb_rows`, `uniform_spacing`, and `covers_cols` / `covers_rows` (whether the
column/row starts summed to exactly `sbCols` / `sbRows`). The `ns()`-bounded loops make
coverage exact for any decodable stream, so the coverage flags are defensive (a sound
check that never false-positives a conformant stream).

Sequence wiring: when `seq_tile_info_present_flag`, read `allow_tile_info_change`,
`seqSbSize = get_seq_sb_size()`, then
`tile_params(max_frame_width_minus_1 + 1, max_frame_height_minus_1 + 1, seqSbSize,
seqSbSize, false)` with the parsed `seq_tier` / `seq_level_idx`.

## 5. Validator

No new sequence/MFH parser dispatch is needed: because the parsers now fully parse
segment and tile info, the existing `is_fully_parsed()` / `unimplemented_at.is_none()`
gates in `SequenceHeaderSyntax`, `MultiFrameHeaderSyntax`, and the `context` MFH/seq
observers automatically run the §5.2.1 payload-tail check and record availability. The
only additions are the local tile semantic diagnostics, emitted from
`SequenceHeaderSyntax` on a fully parsed header that carries tile params:

- `tile-params/tile-cols-out-of-range` — `TileCols > MAX_TILE_COLS` (§6.17.7.2).
- `tile-params/tile-rows-out-of-range` — `TileRows > MAX_TILE_ROWS` (§6.17.7.2).
- `tile-params/nonuniform-cols-do-not-cover-frame` — non-uniform `startSb != sbCols`
  (§6.17.7.3).
- `tile-params/nonuniform-rows-do-not-cover-frame` — non-uniform `startSb != sbRows`
  (§6.17.7.3).

The coverage diagnostics are defensive (structurally satisfied by the `ns()`-bounded
parse); the out-of-range diagnostics are reachable for a non-uniform stream that codes
more than 64 tiles.

## 6. Tables and reserved levels

`Tile_Width_Scaling_Factor` / `Tile_Area_Scaling_Factor` are `[seq_tier][seq_level_idx]`
with defined entries for level indices 0–21 and reserved entries for 22–30; index 31 is
the special unconstrained case. Reserved indices are modeled as `None`. A valid AV2
stream never uses a reserved level, so the only residual bounded tile-params status is
for non-conformant reserved levels, documented in the matrix and validator state.

## 7. Testing strategy

Core unit tests: `read_su` (0/-1 for `su(1)`, multi-bit ±, EOF, invalid width);
`parse_seg_info` (all-disabled 8- and 16-segment, a signed quantizer feature path);
`tile_params` (`tile_log2`, `uniform_spacing`, uniform and non-uniform sequence tile
configs); sequence/MFH composite tests for present-and-fully-parsed segment/tile info;
proptests for no panic on arbitrary input. Validator tests: malformed tail after
segment info now diagnosed; non-uniform `>64` tiles flagged; coverage check unit-tested
via a synthetic `TileParams`. CLI: the repurposed sequence-tile fixture now reports a
fully parsed tile config.
