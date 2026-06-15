# Tasks

## Implementation
- [x] Add `crates/splot-core/src/write/seq_tile.rs`: `write_sequence_filter_config`
      (§ 5.4.10), `write_sequence_tile_config` + `write_tile_params` (§ 5.4.2 / § 5.18.7.3),
      and the composing `write_sequence_header`, each validating fully up front.
- [x] Add `WriteError::UnwritableSequenceHeader { feature }` for a tile-present header at a
      reserved `seq_level_idx`.
- [x] Expose the per-config `check_*_encodable` helpers as `pub(crate)` so
      `write_sequence_header` validates the whole header before any bit.
- [x] Register the module + re-export the public writers in `write/mod.rs`.

## Tests and proof
- [x] Filter + tile semantic round-trip property tests (all branches) via the public parsers.
- [x] Byte-exact unit tests across all seven tile modes; full-header byte-exact tests.
- [x] One rejection test per `WriteError` path (asserting `bit_len()==0`), incl. the
      gated-off filter fields, the reserved-level tile header, grid-mismatch, corrupt
      start array, and the unaligned-writer guard.
- [x] Never-panics property tests. The `tile_round_trips` property test drives
      parse -> write -> parse across all conformant levels (0..=21) and both tiers, so any
      drift between the writer's duplicated §A scaling tables and the parser's private
      copies surfaces as a round-trip failure (the parser keeps its tables private and the
      writer mission keeps the parser read-only, so there is no direct table-equality test).

## Matrix and docs
- [x] Advance `write` `todo -> done` on `AV2-5.4.10-SEQUENCE-FILTER-CONFIG` and
      `AV2-5.4.2-SEQUENCE-TILE-CONFIG`, and `partial -> done` on the
      `AV2-5.4-SEQUENCE-HEADER` umbrella, with proof. Regenerate `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate seq-header-writer-tile --strict`
