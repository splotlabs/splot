# Tasks

## Writer (additive — no model change)
- [x] `write/error.rs`: add `WriteError::NonCanonicalContentInterpretation { what: &'static str }`.
- [x] `write/content_interpretation.rs`: `write_content_interpretation(writer, ci)` inverting
      `parse_content_interpretation` (§ 5.15) + a private `write_timing_info` inverting
      `parse_timing_info`. Reproduce tolerated/reserved values verbatim (reserved_2bit, reserved
      color/aspect/scan idc); reject only the decidable structural inconsistencies (extended_sar vs
      `idc == 255`, color primaries vs `idc == 0`, byte-align, field-width). Re-export in
      `write/mod.rs`.
- [x] `write/dispatch.rs`: route `ParsedObu::ContentInterpretation` to the new writer + the generic
      tail; reject a non-empty passthrough; drop it from the `Unimplemented` arm; update the doc
      counts (nine written / five remaining).

## Tests and proof
- [x] `content_interpretation.rs` writer tests: round-trips for minimal, each sub-struct present,
      `aspect_ratio_idc == 255` (extended SAR) and non-255, a reserved color idc, a reserved aspect
      idc, a non-zero `reserved_2bit`, and the timing-info branch (all must ROUND-TRIP, proving
      verbatim reproduction); reject tests for the decidable invariants. A dispatch round-trip test.

## Matrix and docs
- [x] `AV2-5.15-CONTENT-INTERPRETATION` write `todo` → `done` (+ note); `ENC-BITSTREAM-WRITER` note:
      five unwritten types remain. Regenerate `docs/FEATURE-STATUS.md` (explicit `--output`).

## Checks
- [x] `cargo xtask ci` and `openspec validate content-interpretation-writer --strict`
