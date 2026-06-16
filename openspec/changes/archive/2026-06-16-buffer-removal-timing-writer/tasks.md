# Tasks

## Writer (additive — no model change)
- [x] `write/error.rs`: add `WriteError::NonCanonicalBufferRemovalTiming { what: &'static str }`.
- [x] `write/buffer_removal_timing.rs`: `write_buffer_removal_timing(writer, brt)` inverting
      `parse_buffer_removal_timing` (§ 5.12), reject-before-write (byte-align; `op_times.len()` ==
      `br_ops_cnt`; `index` == loop counter; `decoder_model_present` <-> `br_time_op.is_some()`;
      primitive field-width / `rg` rejects). Re-export in `write/mod.rs`.
- [x] `write/dispatch.rs`: route `ParsedObu::BufferRemovalTiming` to the new writer + the generic
      non-extensible tail; reject a non-empty passthrough; drop it from the `Unimplemented` arm.

## Tests and proof
- [x] `buffer_removal_timing.rs` writer tests: round-trip (write → `parse_buffer_removal_timing`)
      both forms (extended-layer, OPS-dependent with present/absent `br_time_op`); reject tests for
      each constructed-model invariant (count mismatch, bad index, gated-field mismatch); a
      `rg`-range reject. A dispatch round-trip test (`ParsedObu::BufferRemovalTiming` →
      `write_complete_obu` → reparse) and confirm the harness now reports `RoundTripped` (not
      `Unwritable`).

## Matrix and docs
- [x] `AV2-5.12-BUFFER-REMOVAL-TIMING` write `todo` → `done` (+ note + proof); `ENC-BITSTREAM-WRITER`
      note: eight unwritten types remain. Regenerate `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate buffer-removal-timing-writer --strict`
