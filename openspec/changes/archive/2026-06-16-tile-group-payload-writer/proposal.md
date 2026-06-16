# Change: tile-group-payload-writer

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.20-TILE-GROUP-PAYLOAD` (the § 5.20.1 `tile_group_payload()` framing writer; advances its
  `write` stage from `todo` to `partial`)

## Why

The second tile-group slice (after `tile-group-structure-writer`): the § 5.20.1
`tile_group_payload()` per-tile **framing** writer, the inverse of `parse_tile_group_framing`. With
the § 5.19 structure writer it lets `splot` reproduce a whole intra tile group's bytes (structure +
payload), pending the prefix/frame-header composer.

## What changes

- **Writer** (`crates/splot-core/src/write/tile_group.rs`): `write_tile_group_payload`, the inverse
  of `parse_tile_group_framing` (§ 5.20.1, mirror :8553-8640) on the intra (non-bridge) path. For
  each tile in `framing.tiles` order: a non-last tile writes `tile_size_minus_1 = tile_size - 1` as
  `le(TileSizeBytes)` then its coded-tile bytes; the **last** tile writes its coded-tile bytes only
  (no size field, its `tileSize` is the region remainder). The coded-tile bytes are **not** modeled
  by the parser, so they are supplied as a per-tile **passthrough** (`tile_data: &[&[u8]]`, one slice
  per tile) and emitted verbatim — byte-exact, no model change.
- **Reject-before-write** (a reject leaves `bit_len()` unchanged), via the existing
  `WriteError::NonCanonicalTileGroup { what }` and `WriteError::WriterNotByteAligned`:
  - a non-byte-aligned writer (the § 5.20 framing is byte-granular, after the § 5.19
    `byte_alignment()`);
  - a `framing.defect.is_some()` (a defective framing has no faithful byte form);
  - `is_bridge == true` (a bridge frame's tiles read no size field and record `tile_size == 0`,
    unreconstructable — and the intra-complete tile-group path has `IsBridge == 0`);
  - an empty `framing.tiles`, or a `tile_data` length disagreeing with the tile count;
  - a `TileSizeBytes` outside `1..=4` (§ 6.17.7.3);
  - a per-tile `tile_data[i].len()` that disagrees with the recorded `tile_size`;
  - a `tile_size == 0` (a zero-size tile — § 8.2.4 floor; also a non-last `tile_size - 1` would
    underflow);
  - a non-last `tile_size - 1` that does not fit `le(TileSizeBytes)`.
- **Round-trip:** the emitted region reparses (with the matching `tg_start`/`tg_end`/`TileSizeBytes`)
  to an equal `TileGroupFraming` — the recorded `tile_size`s and the recomputed offsets all match,
  with `defect == None` — and the passthrough tile bytes are byte-exact.
- **No model change.** Purely additive; the parser/model are untouched.

## Validator impact

None. No new diagnostics.

## Non-goals

- No `decode_tile()` / § 5.20.2-.10 block syntax, no symbol decode (the coded-tile bytes are opaque).
- No inter / BRU / bridge framing, no prefix or composing `write_tile_group_obu` (following slices).
- No public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write` surface; reuses `NonCanonicalTileGroup`).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (the `AV2-5.20-TILE-GROUP-PAYLOAD` row, `write` → `partial`)
  + regenerated `docs/FEATURE-STATUS.md`.
