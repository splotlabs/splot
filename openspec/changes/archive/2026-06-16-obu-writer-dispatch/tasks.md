# Tasks

## Writer (additive — no model change)
- [x] `write/error.rs`: add `WriteError::Unimplemented { feature: &'static str }` (an honest stub for
      an OBU type with no body writer yet).
- [x] `write/obu.rs` (or a new `write/dispatch.rs`): `write_obu_payload(writer, payload, is_extensible,
      passthrough)` — the per-type body + the `finish_obu_payload` tail (`obu_extension_flag = 0` +
      `trailing_bits()` for a non-empty extensible body), dispatching over `ParsedObu`; the five
      written types (TemporalDelimiter / SequenceHeader / Padding / MetadataShort / MetadataGroup)
      delegate to the existing writers, the other ten return `Unimplemented`. And
      `write_complete_obu(writer, header, payload, passthrough)` = `write_obu_header` +
      `write_obu_payload(.., header.obu_type.is_extensible_obu(), ..)`. Scratch-writer reject-before-write.
      Re-export in `write/mod.rs`; extend the module `//!` doc.

## Tests and proof
- [x] Round-trip per written type via `dispatch_obu_payload`; an `Unimplemented` test for a couple of
      unwritten types; a sub-writer-reject-propagates test (`bit_len() == 0`); a `write_complete_obu`
      → `write_annexb_obu` framed round-trip for the sequence-header + metadata cases.

## Matrix and docs
- [x] Add a WRITER note to `ENC-BITSTREAM-WRITER` recording the dispatch (the five written types; the
      ten unwritten return Unimplemented; the frame-carrying types route separately). Regenerate
      `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate obu-writer-dispatch --strict`
