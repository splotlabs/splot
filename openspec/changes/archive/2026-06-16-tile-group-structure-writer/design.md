# Design: tile-group-structure-writer

## Context

`parse_tile_group_structure` (`crates/splot-core/src/headers/tile_group.rs:257`) parses the § 5.19
`tile_group_obu()` structure that follows the optional `frame_header()`, on the intra path where
`use_bru` / `bru_inactive` are the derived constant `0`. It produces a `TileGroupStructure` with the
syntax fields `tile_start_and_end_present_flag` / `tg_start` / `tg_end`, plus the parse-context
fields `outcome` (`Complete` / `Truncated`), `header_bytes`, and `payload_size`. The bit order is:
`tile_start_and_end_present_flag` `f(1)` (only `NumTiles > 1`); `tg_start` then `tg_end`
`f(tileBits)` (only `NumTiles > 1 && flag`); `byte_alignment()`. `tileBits = TileColsLog2 +
TileRowsLog2` (capped at 32), from `TileGroupLayout::tile_bits`.

## Decisions

- **Invert the structure only; the prefix/frame-header and the § 5.20 payload are separate slices.**
  This slice is the self-contained inverse of `parse_tile_group_structure`. It takes the
  `TileGroupLayout` (carrying `NumTiles` / `TileColsLog2` / `TileRowsLog2`) the parser takes, so it
  derives `tileBits` and the range bounds identically.
- **Emit syntax bits, recompute nothing about offsets.** `header_bytes` / `payload_size` are
  byte-offset artifacts of the whole-OBU parse context (`consumed_bits / 8`, `sz - headerBytes`);
  the structure writer does not own the OBU `sz` or the prefix/frame-header bytes, so it does not
  emit or assert them. The composing OBU writer (a later slice) owns that boundary. The round-trip
  for this slice is therefore **semantic on the syntax fields** plus byte-exact for the emitted
  region; the test reparses the emitted bytes (with a chosen `sz`) and compares
  `tile_start_and_end_present_flag` / `tg_start` / `tg_end` and asserts `outcome == Complete`.
- **`byte_alignment()` via `BitWriter::align_to_byte`.** § 5.19 closes with `byte_alignment()`
  (zero pad, § 6.2.4), the inverse of the parser's `byte_align_zero`. This is the zero-pad helper,
  not `trailing_bits()`.
- **Reject-before-write set (the complete non-serializable surface).** Validated up front so a
  reject leaves `bit_len()` unchanged:
  - `outcome != Complete` (a `Truncated` structure has no faithful byte form) →
    `NonCanonicalTileGroup { what: "incomplete_structure" }`.
  - `NumTiles == 0` (degenerate layout, no decodable tile range) → `"degenerate_layout"`.
  - The flag/range consistency the parser enforces by construction: when the range is *not* read
    (`NumTiles == 1`, or `NumTiles > 1 && !flag`), the model must carry the inferred default
    (`tg_start == 0 && tg_end == NumTiles - 1`); a non-default range there could not have been
    produced and would be silently dropped on reparse → `"inferred_range_mismatch"`. When
    `NumTiles == 1` the flag must be `false` (it is never read) → `"flag_without_multi_tile"`.
  - When the range *is* written, `tg_start` / `tg_end` must fit `f(tileBits)` and satisfy
    `tg_end >= tg_start` → `"tg_range"` (an under-width or inverted range is non-reproducible). The
    `f(tileBits)` fit is also enforced by the primitive, but is checked up front for
    reject-before-write.
  - `tileBits == 0` with the range gated on (a `1x1` tile layout cannot signal a non-trivial range)
    is covered by the `NumTiles > 1` gate (a `NumTiles > 1` layout has `tileBits >= 1`); assert it.
- **`WriteError::NonCanonicalTileGroup { what }`** — one additive, writer-only variant in
  `write/error.rs`, after `NonCanonicalMetadata`, mirroring the per-family precedent. The
  parser/decoder error model is untouched.

## Testing

Round-trip via `parse_tile_group_structure`: build a `TileGroupStructure` + `TileGroupLayout`
(single-tile inferred range; multi-tile with the flag clear → inferred range; multi-tile with the
flag set → explicit `tg_start`/`tg_end` across a range of `tileBits` widths incl. the boundary
values), write it, reparse the emitted bytes with a chosen `sz`, and assert the syntax fields match
and `outcome == Complete`. One reject test per reject path (`bit_len() == 0`). A parser-driven /
constructed round-trip proptest over arbitrary valid `(layout, structure)` and a
never-panics-on-constructed-models proptest over arbitrary field values (incl. out-of-range
`tg_start`/`tg_end`, huge `NumTiles`, degenerate layouts).
