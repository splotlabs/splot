# encoder-tools delta: tile-group-structure-writer

## ADDED Requirements

### Requirement: tile-group structure writer

`splot-core` SHALL provide a writer that is the exact inverse of the § 5.19 `tile_group_obu()`
structure parser (`parse_tile_group_structure`) on the intra path: it SHALL emit
`tile_start_and_end_present_flag` (`f(1)`, only when `NumTiles > 1`), `tg_start` and `tg_end`
(`f(tileBits)`, only when `NumTiles > 1` and the flag is set, with `tileBits = TileColsLog2 +
TileRowsLog2`), and the closing `byte_alignment()` zero pad; when the tile range is inferred it
SHALL emit no range bits. For every structure the writer accepts, reparsing the written bytes SHALL
yield the original on every syntax field (`tile_start_and_end_present_flag`, `tg_start`, `tg_end`)
and a `Complete` outcome. The byte-offset parse-context fields (`header_bytes`, `payload_size`,
`outcome`) are recomputed from the surrounding OBU context and are not emitted by this writer.

The writer SHALL be additive (no model or parser-error change) and SHALL never panic: a structure the
parser could not have produced SHALL be rejected with a typed
`WriteError::NonCanonicalTileGroup` before any bit is written — including a non-`Complete`
(`Truncated`) outcome, a degenerate `NumTiles == 0` layout, a tile range that does not fit
`f(tileBits)` or violates `tg_end >= tg_start`, and a flag/range combination the parser's
inference could not have produced.

#### Scenario: the tile-group structure round-trips

- **WHEN** a parsed `TileGroupStructure` (single-tile inferred range, multi-tile with the flag clear,
  or multi-tile with an explicit `tg_start`/`tg_end`) is written and the emitted bytes are reparsed
  with the same `TileGroupLayout`
- **THEN** the reparsed structure SHALL equal the original on every syntax field with a `Complete`
  outcome, and the emitted region SHALL be byte-exact.

#### Scenario: a non-reproducible tile-group structure is rejected before any bit

- **WHEN** a structure carries a `Truncated` outcome, a degenerate layout, an out-of-range or
  inverted `tg_start`/`tg_end`, or a flag/range combination the parser's inference could not produce
- **THEN** the writer SHALL return a typed `WriteError::NonCanonicalTileGroup` and write no bit.
