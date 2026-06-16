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
- **Inputs.** To keep the signature readable, group the per-tile-group inputs (the `FrameHeaderCore` +
  views + `first_picture_in_tu`, the `TileGroupStructure` + `TileGroupLayout`, and the
  `TileGroupFraming` + `tile_data` + `tile_size_bytes`) — either as several parameters or a small
  `TileGroupObuParts<'_>` borrow struct (implementer's choice; prefer the struct if the parameter
  count is unwieldy). The composer does not own the OBU header / size / trailing bits.
- **Reject set.** `"continuation_unsupported"` for a requested non-first form; otherwise the composer
  is a thin sequencer and every other reject is raised (and surfaced) by the delegated sub-writers,
  whose own reject-before-write guarantees compose through the scratch buffer.

## Testing

Round-trip the whole first-tile-group OBU payload: build a valid `FrameHeaderCore` + views (reuse the
`write_frame_header_core` test fixtures / a parsed intra frame header) plus a `TileGroupStructure` +
`TileGroupLayout` consistent with the header's `tile_info()`, and a `TileGroupFraming` + `tile_data`,
write the OBU payload, then reparse it stage by stage — `parse_tile_group_prefix` (assert
`is_first_tile_group` + `frame_header_present_flag`), the frame-header core, `parse_tile_group_structure`,
and `parse_tile_group_framing` — and assert each stage's syntax fields round-trip. One reject test for
the `continuation_unsupported` path and one showing a sub-writer reject propagates with `bit_len() == 0`.
A never-panics-on-constructed-models proptest over the composer.
