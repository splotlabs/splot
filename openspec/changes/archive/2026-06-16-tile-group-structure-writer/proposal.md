# Change: tile-group-structure-writer

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.19-TILE-GROUP` (the § 5.19 `tile_group_obu()` structure writer; advances its `write`
  stage from `todo` to `partial`)

## Why

The next `obu-payload-writers-*` family after sequence-header, frame-header, and metadata: the
**tile group**. This first slice inverts the § 5.19 `tile_group_obu()` *structure* — the
`tile_start_and_end_present_flag` / `tg_start` / `tg_end` tile-range fields and the closing
`byte_alignment()` — the bits that follow the optional `frame_header()`. The § 5.20.1 per-tile
payload framing (tile sizes + opaque tile data) and the prefix + frame-header composition are
separate following slices (`tile-group-payload-writer`, then a composer).

## What changes

- **Writer** (`crates/splot-core/src/write/tile_group.rs`): `write_tile_group_structure`, the exact
  inverse of `parse_tile_group_structure` (§ 5.19, mirror :8465-8527) on the intra path. It emits
  `tile_start_and_end_present_flag` `f(1)` (only when `NumTiles > 1`), `tg_start` / `tg_end`
  `f(tileBits)` (only when `NumTiles > 1 && flag`, where `tileBits = TileColsLog2 + TileRowsLog2`
  from the `TileGroupLayout`), and the closing `byte_alignment()` (zero pad). When the range is
  inferred (`NumTiles == 1` or the flag is `0`) it writes no range bits, matching the parser's
  `0 .. NumTiles - 1` inference.
- **Reject-before-write:** the structure is validated up front (a reject leaves `bit_len()`
  unchanged). Rejected: a non-`Complete` `outcome` (`Truncated`); a `tg_start` / `tg_end` that does
  not fit `f(tileBits)` or violates `tg_end >= tg_start`; a flag/range combination the parser could
  not have produced (the flag is `false` but the range is non-default, or `NumTiles == 1` with a set
  flag); and a degenerate `NumTiles == 0` layout. A new additive, writer-only
  `WriteError::NonCanonicalTileGroup { what }` variant carries the reason (mirroring
  `NonCanonicalMetadata` / `NonCanonicalFrameHeader`).
- **Parse-context artifacts are not emitted.** `header_bytes` / `payload_size` (derived from
  `consumed_bits` + the OBU `sz`) and `outcome` are recomputed by a reparse from the surrounding OBU
  context; the structure writer emits only the syntax bits, so the round-trip is **semantic** on the
  syntax fields (`tile_start_and_end_present_flag`, `tg_start`, `tg_end`) and **byte-exact** for the
  emitted structure region. The `header_bytes` / `payload_size` boundary is owned by the OBU/composer
  writer (a following slice).
- **No model change.** Purely additive; the parser and the tile-group models are untouched.

## Validator impact

None. No new diagnostics.

## Non-goals

- No § 5.20.1 per-tile payload framing or tile-data passthrough (the `tile-group-payload-writer`
  slice).
- No prefix (`is_first_tile_group` / `frame_header_present_flag`) or embedded `frame_header()`
  emission, and no composing `write_tile_group_obu` (a following slice; the embedded-frame-header
  input is decided there).
- No inter / BRU / bridge paths (intra structure only — the parser's documented intra precondition).
- No public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write` module + one additive `WriteError` variant).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (the `AV2-5.19-TILE-GROUP` row, `write` → `partial`) +
  regenerated `docs/FEATURE-STATUS.md`.
