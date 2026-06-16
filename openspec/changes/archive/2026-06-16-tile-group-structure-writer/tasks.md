# Tasks

## Writer (additive — no model change)
- [x] `write/error.rs`: add `WriteError::NonCanonicalTileGroup { what: &'static str }` (after
      `NonCanonicalMetadata`), with a doc comment + spec citation.
- [x] `write/tile_group.rs`: `write_tile_group_structure(writer, structure, layout)` — the inverse
      of `parse_tile_group_structure` (§ 5.19): `tile_start_and_end_present_flag` `f(1)` gated on
      `NumTiles > 1`, `tg_start` / `tg_end` `f(tileBits)` gated on `NumTiles > 1 && flag`, then
      `byte_alignment()`. An up-front `check_*_encodable` enforces the reject set (reject-before-write,
      `bit_len() == 0`). Register + re-export in `write/mod.rs`; extend the module `//!` doc.

## Tests and proof
- [x] Round-trip tests (single-tile inferred, multi-tile flag-clear inferred, multi-tile flag-set
      explicit across `tileBits` widths incl. boundaries) via `parse_tile_group_structure`; one
      reject test per reject path (`bit_len() == 0`); a constructed round-trip proptest + a
      never-panics-on-constructed-models proptest.

## Matrix and docs
- [x] Advance `write` on `AV2-5.19-TILE-GROUP` from `todo` to `partial` (note the structure-only
      scope; the § 5.20 payload + composer remain). Regenerate `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate tile-group-structure-writer --strict`
