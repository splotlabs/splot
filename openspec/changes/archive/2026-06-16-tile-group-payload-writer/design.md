# Design: tile-group-payload-writer

## Context

`parse_tile_group_framing` (`crates/splot-core/src/headers/tile_group.rs:497`) walks the § 5.20.1
`tile_group_payload()` loop over `tg_start ..= tg_end`. For each tile: the **last** tile takes the
remaining `sz` and reads no size field (mirror :8555-8557); a non-last **bridge** tile reads no size
field and records `tile_size == 0` (unframeable, mirror :8559); a non-last **non-bridge** tile reads
`tile_size_minus_1 le(TileSizeBytes)` (mirror :8565), sets `tileSize = tile_size_minus_1 + 1` (mirror
:8569), and advances `sz -= tileSize + TileSizeBytes` (mirror :8571). It records each tile's
`TileFraming` (byte offsets + `tile_size`) and stops at the first provable `TileFramingDefect`. The
coded-tile **bytes themselves are not modeled** — only offsets and sizes.

## Decisions

- **Invert the framing; tile bytes are a per-tile passthrough.** `write_tile_group_payload` takes the
  `TileGroupFraming` (the per-tile `tile_size`s) and a `tile_data: &[&[u8]]` with one coded-tile slice
  per tile, and emits, in `framing.tiles` order: for a non-last tile, `tile_size - 1` as
  `le(TileSizeBytes)` (via `BitWriter::write_le_u64`) then `tile_data[i]`; for the last tile,
  `tile_data[i]` only. This is the exact inverse of the parser's loop. No model change (the parser
  deliberately does not model tile bytes, tile_group.rs:171/334-351), exactly like the metadata blob
  passthroughs.
- **Offsets are recomputed, not emitted.** `size_field_offset` / `tile_data_offset` are byte cursors
  the parser derives; the writer lays tiles sequentially from the region start, so a reparse recomputes
  identical offsets. The round-trip is semantic on the whole `TileGroupFraming` (sizes + recomputed
  offsets + `defect == None`) and byte-exact for the emitted region.
- **Intra / non-bridge only; reject `is_bridge`.** A bridge tile's `tile_size` is recorded `0`
  (unframeable), so a bridge framing cannot be reconstructed from the model — and the intra-complete
  tile-group path the structure writer targets has `IsBridge == 0`. The writer takes `is_bridge` for
  symmetry with the parser and rejects `true`.
- **Reject-before-write set** (each → `WriteError::NonCanonicalTileGroup { what }`, except the
  alignment guard → `WriterNotByteAligned`), validated before the first write so a reject leaves
  `bit_len()` unchanged: a non-byte-aligned writer (`"..."` -> `WriterNotByteAligned`); a
  `framing.defect.is_some()` (`"framing_defect"`); `is_bridge` (`"bridge_unframeable"`); an empty
  `framing.tiles` (`"empty_framing"`); `tile_data.len() != framing.tiles.len()` (`"tile_data_count"`);
  `tile_size_bytes` outside `1..=4` (`"tile_size_bytes_domain"`); a per-tile
  `tile_data[i].len() as u64 != tiles[i].tile_size` (`"tile_data_len"`); a `tile_size == 0`
  (`"zero_size_tile"` — the § 8.2.4 floor, and a non-last `tile_size - 1` would underflow); a non-last
  `tile_size - 1` outside `le(TileSizeBytes)` i.e. `>= 1u64 << (8 * TileSizeBytes)`
  (`"tile_size_field_overflow"`).
- **No panic on constructed models.** `tile_size - 1` is guarded by the `tile_size == 0` reject;
  `1u64 << (8 * tile_size_bytes)` is bounded by the `1..=4` domain reject (`8*4 == 32 < 64`);
  `write_le_u64` is fed a validated value/width; the tile loop is bounded by `framing.tiles.len()`.

## Testing

Round-trip via `parse_tile_group_framing`: build a `TileGroupFraming` + per-tile `tile_data`
(single-tile; multi-tile across `TileSizeBytes` 1..=4 incl. a max-`tile_size` boundary; a tile whose
`tile_size_minus_1` spans the full `le(TileSizeBytes)` width), write it, reparse the emitted region
with the matching `tg_start`/`tg_end`/`TileSizeBytes`, and assert the framing equals (sizes +
recomputed offsets + `defect == None`) and the tile bytes are byte-exact. One reject test per reject
path (`bit_len() == 0`). A constructed round-trip proptest + a never-panics-on-constructed-models
proptest (arbitrary `tile_size`s, counts, `TileSizeBytes`, `is_bridge`, defects).
