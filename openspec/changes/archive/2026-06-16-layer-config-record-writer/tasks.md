# Tasks

## Writer (additive — no model change)
- [x] `write/error.rs`: add `WriteError::NonCanonicalLayerConfigRecord { what: &'static str }`.
- [x] `write/layer_config_record.rs`: `write_layer_config_record(writer, record)` inverting
      `parse_layer_config_record` (§ 5.8) and the § 5.8.1–5.8.9 sub-struct parsers. Branch on the
      `Global` / `Local` variant; reuse `align_to_byte` for `byte_alignment()`. Reject-before-write
      (gated `Option`s vs flags, set-bit-derived `Vec` lengths/ids, the atlas-vs-embedded mutual
      exclusion, the `lcr_global_payload` filler invariant, field-width domains); reproduce tolerated
      reserved/descriptive values verbatim. Re-export in `write/mod.rs`.
- [x] `write/dispatch.rs`: route `ParsedObu::LayerConfigurationRecord` to the new writer + the generic
      extensible tail; reject a non-empty passthrough; drop it from the `Unimplemented` arm; update the
      doc counts (thirteen written / one remaining).

## Tests and proof
- [x] `layer_config_record_tests.rs` (sibling `include!` file): round-trips (parse a hand-built
      payload → write → reparse → assert_eq) for a minimal global LCR, a global with aggregate / PTL /
      atlas-id / payload (exact-size and with remaining filler bits), a global payload with a
      dependent-xlayer map, a minimal local LCR, a local with a local-atlas embedded layer carrying
      color + aux + explicit-view + max-expected resolution; reject tests for each decidable invariant.
      A dispatch round-trip test. A `roundtrip_obu_bytes` fuzz smoke confirming no over-rejection.

## Matrix and docs
- [x] `AV2-5.8-LAYER-CONFIG-RECORD` write `todo` → `done` (+ note); `ENC-BITSTREAM-WRITER` note: one
      unwritten type remains (`QuantizationMatrix`). Regenerate `docs/FEATURE-STATUS.md` (explicit
      `--output`).

## Checks
- [x] `cargo xtask ci` and `openspec validate layer-config-record-writer --strict`
