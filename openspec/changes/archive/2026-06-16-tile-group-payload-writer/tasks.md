# Tasks

## Writer (additive — no model change)
- [x] `write/tile_group.rs`: `write_tile_group_payload(writer, framing, tile_data, tile_size_bytes,
      is_bridge)` — the inverse of `parse_tile_group_framing` (§ 5.20.1): non-last tiles write
      `tile_size - 1` as `le(TileSizeBytes)` then the passthrough coded-tile bytes; the last tile
      writes its bytes only. An up-front `check_*_encodable` enforces the reject set
      (reject-before-write, `bit_len() == 0`). Reuse `WriteError::NonCanonicalTileGroup` /
      `WriterNotByteAligned`. Re-export in `write/mod.rs`; extend the module `//!` doc.

## Tests and proof
- [x] Round-trip tests (single-tile; multi-tile across `TileSizeBytes` 1..=4 incl. a full-width
      `tile_size_minus_1`) via `parse_tile_group_framing`; one reject test per reject path
      (`bit_len() == 0`); a constructed round-trip proptest + a never-panics-on-constructed proptest.

## Matrix and docs
- [x] Advance `write` on `AV2-5.20-TILE-GROUP-PAYLOAD` from `todo` to `partial` (note the framing /
      passthrough scope; the `decode_tile()` block syntax + bridge/BRU arms remain). Regenerate
      `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate tile-group-payload-writer --strict`
