# encoder-tools delta: tile-group-payload-writer

## ADDED Requirements

### Requirement: tile-group payload framing writer

`splot-core` SHALL provide a writer that is the inverse of the § 5.20.1 `tile_group_payload()`
framing parser (`parse_tile_group_framing`) on the intra (non-bridge) path: for each tile in order,
a non-last tile SHALL emit `tile_size_minus_1 = tile_size - 1` as `le(TileSizeBytes)` followed by its
coded-tile bytes, and the last tile SHALL emit its coded-tile bytes only (no size field). The
coded-tile bytes are not modeled by the parser, so the writer SHALL accept them as a per-tile
passthrough input and emit them verbatim. For every framing the writer accepts, reparsing the emitted
region with the matching `tg_start` / `tg_end` / `TileSizeBytes` SHALL yield an equal
`TileGroupFraming` (the per-tile `tile_size`s and recomputed offsets, with no defect), and the tile
bytes SHALL be byte-exact.

The writer SHALL be additive (no model or parser-error change) and SHALL never panic: a framing the
parser could not have produced — a defective framing, a bridge framing (unreconstructable
`tile_size == 0`), a tile count or passthrough length mismatch, a `TileSizeBytes` outside `1..=4`, a
zero-size tile, or a `tile_size - 1` outside `le(TileSizeBytes)` — SHALL be rejected with a typed
`WriteError` before any bit is written, and a non-byte-aligned writer SHALL be rejected with
`WriteError::WriterNotByteAligned`.

#### Scenario: the tile-group payload framing round-trips

- **WHEN** a parsed (or constructed) `TileGroupFraming` and its per-tile coded-tile bytes are written
  and the emitted region is reparsed with the same `tg_start` / `tg_end` / `TileSizeBytes`
- **THEN** the reparsed framing SHALL equal the original (sizes + recomputed offsets, no defect) and
  the coded-tile bytes SHALL be byte-exact.

#### Scenario: a non-reproducible tile-group framing is rejected before any bit

- **WHEN** a framing carries a defect, a bridge tile, a tile-data length or count mismatch, an
  out-of-range `TileSizeBytes`, a zero-size tile, or a `tile_size - 1` exceeding `le(TileSizeBytes)`
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
