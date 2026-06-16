# Tasks

## Writer (additive — no model change)
- [x] `write/error.rs`: add `WriteError::NonCanonicalAtlasSegment { what: &'static str }`.
- [x] `write/atlas_segment.rs`: `write_atlas_segment(writer, atlas)` inverting `parse_atlas_segment`
      (§ 5.9 / § 5.9.1–5.9.5), reject-before-write (mode-vs-mode_info, derived num_segments, gated
      Options, count-vs-len, field-width); reproduce the § 6.9.2 descriptive id values verbatim.
      Re-export in `write/mod.rs`.
- [x] `write/dispatch.rs`: route `ParsedObu::AtlasSegment` to the new writer + the generic tail;
      reject a non-empty passthrough; drop it from the `Unimplemented` arm; update the doc counts
      (eleven written / three remaining).

## Tests and proof
- [x] `atlas_segment.rs` writer tests: round-trips (parse a hand-built payload → write → reparse →
      assert_eq) for each of the five modes, signaled / non-signaled label forms, and uniform /
      explicit region dims; reject tests for each decidable invariant. A dispatch round-trip test. A
      `roundtrip_obu_bytes` fuzz smoke confirming no over-rejection.

## Matrix and docs
- [x] `AV2-5.9-ATLAS-SEGMENT` (+ `AV2-5.9.1-ATLAS-LABEL-SEGMENT-INFO`) write `todo` → `done` (+ note); `ENC-BITSTREAM-WRITER`
      note: three unwritten types remain. Regenerate `docs/FEATURE-STATUS.md` (explicit `--output`).

## Checks
- [x] `cargo xtask ci` and `openspec validate atlas-segment-writer --strict`
