# Tasks

## Writer (additive — no model change)
- [x] `write/error.rs`: add `WriteError::NonCanonicalMsdo { what: &'static str }`.
- [x] `write/msdo.rs`: `write_msdo(writer, msdo)` inverting `parse_msdo` (§ 5.6), reject-before-write
      (byte-align; gated `multistream_large_picture_idc` vs `multistream_even_allocation_flag`;
      `sub_stream_count` == `num_streams_minus_2 + 2`; non-zero unused `sub_streams[count..]`;
      primitive field-width rejects). Re-export in `write/mod.rs`.
- [x] `write/dispatch.rs`: route `ParsedObu::Msdo` to `write_msdo` + the generic tail; reject a
      non-empty passthrough; drop it from the `Unimplemented` arm; update the six→seven written /
      eight→seven unwritten doc counts.

## Tests and proof
- [x] `msdo.rs` writer tests: round-trip (write → `parse_msdo`) for even-allocation and
      non-even-allocation forms with the full substream count; reject tests for each constructed-model
      invariant (gated large-picture mismatch, substream-count mismatch, non-zero unused slot,
      field-width). A dispatch round-trip test (`ParsedObu::Msdo` → `write_obu_payload` → reparse).

## Matrix and docs
- [x] `AV2-5.6-MSDO` write `todo` → `done` (+ note + proof); `ENC-BITSTREAM-WRITER` note: seven
      unwritten types remain. Regenerate `docs/FEATURE-STATUS.md` (explicit `--output`).

## Checks
- [x] `cargo xtask ci` and `openspec validate msdo-writer --strict`
