# Tasks

## Writer (additive — no model change)
- [x] `write/error.rs`: add `WriteError::NonCanonicalMultiFrameHeader { what: &'static str }`.
- [x] `write/multi_frame_header.rs`: `write_multi_frame_header(writer, mfh)` inverting
      `parse_multi_frame_header` (§ 5.7), reusing `write_seg_info` for `segment_info`.
      Reject-before-write (apply-deblocking forced-false vs update, seg-info Options vs the stored
      flag, frame-size bit-width range, field-width); reproduce tolerated id values verbatim.
      Re-export in `write/mod.rs`.
- [x] `write/dispatch.rs`: route `ParsedObu::MultiFrameHeader` to the new writer + the generic tail;
      reject a non-empty passthrough; drop it from the `Unimplemented` arm; update the doc counts
      (twelve written / two remaining).

## Tests and proof
- [x] `multi_frame_header.rs` writer tests: round-trips (parse a hand-built payload → write → reparse
      → assert_eq) for minimal, frame-size present (small + max bit widths), deblocking-update flag
      combos, and seg-info present with ext_seg true and false; reject tests for each decidable
      invariant. A dispatch round-trip test. A `roundtrip_obu_bytes` fuzz smoke confirming no
      over-rejection.

## Matrix and docs
- [x] `AV2-5.7-MULTI-FRAME-HEADER` write `todo` → `done` (+ note); `ENC-BITSTREAM-WRITER` note: two
      unwritten types remain. Regenerate `docs/FEATURE-STATUS.md` (explicit `--output`).

## Checks
- [x] `cargo xtask ci` and `openspec validate multi-frame-header-writer --strict`
