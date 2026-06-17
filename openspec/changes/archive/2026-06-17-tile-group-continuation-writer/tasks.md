# Tasks

## Parser accessor (additive — read-only data exposure)
- [x] `headers/tile_group.rs`: make `RecordedFrameHeaderBits::bit(&self, index: u64) -> Option<bool>`
      public so the writer can re-emit `frame_header_copy()` verbatim (no parse-behavior change).

## Writer (additive — no model change)
- [x] `write/tile_group.rs`: `write_tile_group_continuation_obu(writer, recorded,
      frame_header_present_flag, layout, tile_size_bytes, structure, framing, tile_data, is_bridge)`
      inverting the `is_first_tile_group == 0` path: `is_first_tile_group = 0` `f(1)`,
      `frame_header_present_flag` `f(1)`, the recorded `NumFrameHeaderBits` copy bits when present, then
      `write_tile_group_structure` (no `tg_start == 0` rule) + `write_tile_group_payload`. Scratch
      writer, reject-before-write. Re-export in `write/mod.rs`.

## Tests and proof
- [x] `tile_group` tests: a round-trip for a non-first tile group — build a payload with a recorded
      first header + `frame_header_present_flag` true and false, parse the prefix / structure / framing
      pieces, write via the continuation composer, reparse the pieces, and assert byte-exact + the
      pieces match. A `tg_start > 0` continuation (the rule the first-group composer forbids). Reject
      tests for the flag-vs-recorded mismatch and the delegated sub-writer rejects.

## Matrix and docs
- [x] `AV2-5.19-TILE-GROUP` writer note: the non-first continuation composer landed (`ENC-BITSTREAM-WRITER`
      note too). Regenerate `docs/FEATURE-STATUS.md` if a status field changes.

## Checks
- [x] `cargo xtask ci` and `openspec validate tile-group-continuation-writer --strict`
