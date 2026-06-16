# Tasks

## Writer (additive — no model change)
- [x] `write/tile_group.rs`: `write_tile_group_obu` (first-tile-group form) — emits
      `is_first_tile_group` `f(1) = 1`, the `frame_header()` via `write_frame_header_core`, then
      `write_tile_group_structure` and `write_tile_group_payload`, in § 5.19 read order. Draft into a
      scratch `BitWriter`, append on full success (reject-before-write, `bit_len() == 0`). Reject the
      non-first form (`"continuation_unsupported"`). Re-export in `write/mod.rs`; extend the module
      `//!` doc.

## Tests and proof
- [x] A whole-OBU-payload round-trip (build a valid `FrameHeaderCore` + views + structure + framing,
      write, reparse stage-by-stage, assert syntax fields); a `continuation_unsupported` reject test;
      a sub-writer-reject-propagates test (`bit_len() == 0`); a never-panics proptest.

## Matrix and docs
- [x] Add a WRITER note to `AV2-5.19-TILE-GROUP` recording the composing `write_tile_group_obu`
      (first-tile-group form; the continuation + OBU framing remain). Regenerate
      `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate tile-group-obu-composer --strict`
