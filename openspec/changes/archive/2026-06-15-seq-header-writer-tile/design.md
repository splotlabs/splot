# Design: seq-header-writer-tile

## Context

The filter config (§ 5.4.10) and tile config (§ 5.4.2) are the last two configs in
`sequence_header_obu()`, read mid-byte after the § 5.4.3–5.4.8 cascade. The tile config
calls `tile_params()` (§ 5.18.7.3), whose layout is derived from the frame dimensions,
the superblock size, and the level/tier scaling tables. The composing
`write_sequence_header` ties the merged general-fields, config-cascade, filter, and tile
writers into the full OBU body.

## Decisions

- **`write_tile_params` re-derives the grid forward, mirroring the parser.** The parser
  derives `SeqSbColStarts` / `SeqSbRowStarts` from the signaled increment runs; the
  writer re-runs that derivation forward from the persisted starts to recover the exact
  bits — uniform (a `tile_log2` increment-run with the no-stop-bit-at-max edge) or
  non-uniform (per-column width runs, per-row height runs) — and checks the recomputed
  grid equals the model's stored starts before emitting. The level/tier
  `Tile_Width_Scaling_Factor` / `Tile_Area_Scaling_Factor` tables are duplicated locally
  in `write/` (the parser's copies are private); a test asserts they match the parser
  entry-for-entry.
- **Reserved levels are unwritable by construction.** A reserved `seq_level_idx` has no
  defined `MaxTileWidthSb` / `MaxTileAreaSb`, so a tile-present header at a reserved level
  cannot be re-derived. The writer rejects it before any bit with the new
  `WriteError::UnwritableSequenceHeader { feature }` (the § 5.4.2 bounded residual). A
  tile-absent header at a reserved level still writes (no `tile_params` body).
- **`write_sequence_header` validates the whole header up front, then emits in order.**
  It recomputes the parser's derivations (`monochrome` / `single_picture` / `seq_sb_size`
  / `TileParamsInput`) rather than trusting model-stored copies, validates every config
  via the `pub(crate)` `check_*_encodable` helpers (reject-before-write across the whole
  header, including every gated-off non-default field), then writes
  general → partition → segment → intra → inter → scc → tq → filter → tile →
  `film_grain_params_present_flag`. It asserts the writer is byte-aligned on entry and
  emits no OBU header / trailing bits (the OBU framer owns those).
- **Filter config reject-before-write, including gated-off fields.** Every § 5.4.10 field
  is pre-validated, and every `if gate { write field }` has the matching
  `!gate && field != inferred_default` rejection (the GDF/CCSO unit flags, the
  loop-restoration subfields, the adaptive `cdef_on_skip_txfm`), so a constructed model
  with a non-default gated-off field is rejected before any bit.

## Testing

Filter and tile: semantic round-trip property tests over parser-reachable models across
every branch (via the public parsers), byte-exact unit tests (all seven tile modes:
absent, uniform single / two-column / increment-run-at-max, non-uniform wide-first /
two-column / two-row), and one rejection test per `WriteError` path (asserting
`bit_len()==0`), including the gated-off filter fields, the reserved-level tile header,
the grid-mismatch and corrupt-start-array paths, and the unaligned-writer guard. The
composing `write_sequence_header`: full-header byte-exact tests (still-picture no-tile and
tiled headers) plus a round-trip helper, and a never-panics property test.
