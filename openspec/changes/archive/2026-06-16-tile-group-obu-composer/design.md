# Design: tile-group-obu-composer

## Context

`parse_tile_group_prefix` (`crates/splot-core/src/headers/tile_group.rs:61`) reads
`is_first_tile_group` `f(1)`, infers `frame_header_present_flag = 1` for the first group (else reads
it), and — for the first group — parses the embedded `frame_header()`. The deeper `frame_header_core`
is parsed separately (with sequence/MFH state) into a `FrameHeaderCore`; the writer for it,
`write_frame_header_core` (intra path), already exists and is the exact inverse. The § 5.19 structure
(`write_tile_group_structure`) and the § 5.20.1 payload framing (`write_tile_group_payload`) writers
also already exist. This slice only **sequences** them for the first-tile-group form.

## Decisions

- **First tile group only; option A for the frame header.** The composer emits the
  `is_first_tile_group == 1` form: `is_first_tile_group` `f(1) = 1`, then `frame_header()` via
  `write_frame_header_core` (taking the caller's already-built `FrameHeaderCore` + `CoreSeqView` +
  `Option<&MfhFrameView>` + `first_picture_in_tu` — option A from the audit), then the structure, then
  the payload. The `frame_header_present_flag` is the inferred `1` (no bit) for the first group. The
  non-first `frame_header_copy()` continuation is deferred (rejected here).
- **Sequencing order is the parser's order.** `frame_header()` ends mid-byte; the § 5.19 structure
  writer emits its `tile_start_and_end_present_flag` / `tg_start` / `tg_end` bits then its closing
  `byte_alignment()` (so the writer is byte-aligned after it); the § 5.20.1 payload writer then runs
  byte-aligned (its own byte-alignment guard holds). The composer therefore does NOT insert any
  alignment itself — it relies on the structure writer's `byte_alignment()`.
- **Scratch-writer composition.** The composer drafts the whole OBU payload into a local `BitWriter`
  and `append`s to the caller only on full success, so any sub-writer reject (frame-header,
  structure, or payload) leaves the caller's `writer` untouched — reject-before-write for the whole
  composition, exactly as `write_frame_header_core` composes its sub-structures.
- **Inputs — layout / `TileSizeBytes` are DERIVED, not caller-supplied (post-review).** The composer
  takes the `FrameHeaderCore` + views + `first_picture_in_tu`, the `TileGroupStructure`, and the
  `TileGroupFraming` + `tile_data`. It does **not** take `TileGroupLayout` or `tile_size_bytes` as
  inputs: both are derived from `core.tile_info` (§ 5.18.7.2) so the § 5.20.1 framing always stays
  consistent with the bits `write_frame_header_core` emits — an independently-supplied layout /
  `TileSizeBytes` could desync the round-trip (a reparse frames the payload from the header-derived
  values). The composer does not own the OBU header / size / trailing bits.
- **Reject set.** Before any bit: `WriterNotByteAligned` (an OBU payload starts byte-aligned);
  `"continuation_unsupported"` (a requested non-first form); `"not_tile_group_obu"` (a non-tile-group
  `core.obu_type`, e.g. a SEF / TIP header); `"first_tg_start_not_zero"` (the § 6.18 first-group rule);
  `"framing_range_mismatch"` (`framing.tiles.len()` vs the structure's `tg_end - tg_start + 1` tile
  count); `"missing_tile_info"` (no `core.tile_info` to derive from); plus every reject the delegated
  frame-header / structure / payload sub-writers raise, which compose through the scratch buffer.

## Testing

Round-trip the whole first-tile-group OBU payload: build a valid `FrameHeaderCore` + views (reuse the
`write_frame_header_core` test fixtures / a parsed intra frame header) plus a `TileGroupStructure` +
`TileGroupLayout` consistent with the header's `tile_info()`, and a `TileGroupFraming` + `tile_data`,
write the OBU payload, then reparse it stage by stage — `parse_tile_group_prefix` (assert
`is_first_tile_group` + `frame_header_present_flag`), the frame-header core, `parse_tile_group_structure`,
and `parse_tile_group_framing` — and assert each stage's syntax fields round-trip. One reject test for
the `continuation_unsupported` path and one showing a sub-writer reject propagates with `bit_len() == 0`.
A never-panics-on-constructed-models proptest over the composer.
