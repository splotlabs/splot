# Change: seq-header-writer-tile

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.4.10-SEQUENCE-FILTER-CONFIG`, `AV2-5.4.2-SEQUENCE-TILE-CONFIG`
  (each advances its `write` stage `todo -> done`)
- `AV2-5.4-SEQUENCE-HEADER` (the composing `write_sequence_header` completes the
  umbrella `write` stage `partial -> done`)

## Why

This is the final sequence-header writer slice. On top of the merged general-fields
writer (`seq-header-writer-general`) and config-cascade writer
(`seq-header-writer-configs`), it adds the inverse of the two remaining configs — the
filter config (§ 5.4.10) and the tile config (§ 5.4.2, including the shared
§ 5.18.7.3 `tile_params`) — and the composing `write_sequence_header`, which emits the
whole `sequence_header_obu()` body. After this change the sequence-header writer is
complete and round-trips end to end.

## What changes

- Add `crates/splot-core/src/write/seq_tile.rs`:
  - `write_sequence_filter_config` — the inverse of the § 5.4.10 parser (the
    `seq_sb_size`-gated GDF/CCSO unit flags, the loop-restoration mirrored-UV subfield
    chain, the single-picture `cdef_on_skip_txfm` adaptive gate, and
    `df_par_bits_minus_2`), validating every field and every gated-off non-default
    field before any bit.
  - `write_sequence_tile_config` + `write_tile_params` — the inverse of the § 5.4.2 /
    § 5.18.7.3 parser: the `seq_tile_info_present_flag` gate, the uniform increment-run
    encoding (including the no-stop-bit-at-max edge), and the non-uniform width/height
    run loops re-derived from the persisted `SeqSbColStarts` / `SeqSbRowStarts` grid
    against the level/tier `Tile_Width_Scaling_Factor` / `Tile_Area_Scaling_Factor`
    tables.
  - `write_sequence_header` — the composing top-level writer. It recomputes the
    parser's derivations (`monochrome` / `single_picture` / `seq_sb_size` /
    `TileParamsInput`), validates the whole header up front, and emits
    general → partition → segment → intra → inter → scc → tq → filter → tile →
    `film_grain_params_present_flag` in § 5.4.1 order. It writes no OBU header and no
    trailing bits (the caller frames the OBU).
- Add `WriteError::UnwritableSequenceHeader { feature }`: a tile-present header at a
  reserved `seq_level_idx` has no defined tile layout (the § 5.4.2 bounded residual),
  so it is rejected before any bit.
- Expose the per-config `check_*_encodable` helpers as `pub(crate)` so
  `write_sequence_header` can validate the whole header before writing.
- The module stays **additive**: no parser/model/parser-error edits (the only
  non-test changes outside `write/` are the new error variant and `pub(crate)`
  visibility on existing check helpers).

## Validator impact

None. No new diagnostics; the validator is unchanged.

## Non-goals

- No frame/tile-group/metadata/payload writers, no Annex B muxer — later changes.
- No encoder rate decisions; no public `encode` CLI.

## Impact

- Crate: `crates/splot-core` (additive `write` module + one new `WriteError` variant).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (+ regenerated `docs/FEATURE-STATUS.md`).
