# Design: obu-writer-dispatch

## Context

`dispatch_obu_payload` (`crates/splot-core/src/obu.rs:230`) parses an OBU payload into one of
`ParsedObu`'s 15 variants and then calls `finish_obu_payload` (`obu.rs:413`) for the tail: a
non-empty payload of an extensible OBU type reads `obu_extension_flag` (a § 6.2.1 conformance `0`)
then `trailing_bits()`; an empty payload (temporal delimiter) reads nothing. The per-structure
writers already exist for the sequence header (`write_sequence_header`) and metadata
(`write_metadata_short_obu` / `write_metadata_group_obu`), and `write_obu_header` / `write_trailing_bits`
exist. This slice composes them into the inverse of `dispatch_obu_payload` + `finish_obu_payload`.

## Decisions

- **Two functions.** `write_obu_payload(payload, is_extensible, passthrough)` writes the typed body
  then the `finish_obu_payload` tail; `write_complete_obu(header, payload, passthrough)` prepends
  `write_obu_header` and derives `is_extensible` from `header.obu_type.is_extensible_obu()`. The OBU
  *size* / Annex B framing stays with `write_annexb_obu` (which already takes complete-OBU bytes), so
  this slice owns the payload + tail, not the length prefix.
- **The tail is the inverse of `finish_obu_payload`.** After the body: if the body is non-empty and
  `is_extensible`, emit `obu_extension_flag = 0` (`f(1)`) then `trailing_bits()` (via
  `BitWriter::write_trailing_bits` over the bits remaining to the byte/payload boundary); an empty
  body (temporal delimiter) emits nothing. The implementer must read `finish_obu_payload` fully to
  reproduce the exact tail (the `trailing_bits` length and the extensible-vs-not split) and confirm
  against `ObuType::is_extensible_obu`.
- **Padding is opaque + a trailing split.** `PaddingObu` carries `{padding_len, trailing_len}` only
  (the bytes are not modeled); the writer takes the `padding_len` opaque bytes via `passthrough` and
  emits them, then `trailing_len` of `trailing_bits()`, matching the § 5.16 split. (An `obuPayloadSize`
  of 0 emits nothing; 1 emits a single `trailing_bits()` byte.)
- **Honest partial coverage.** The ten unwritten variants return
  `WriteError::Unimplemented { feature: "<matrix-id>" }` — a new additive variant mirroring the
  parser's `Error::Unimplemented`. This is reachable only for those types; the fuzz harness filters
  on `Ok`, and the cross-tool minimal stream uses only the written types. Each unwritten arm cites the
  matrix Feature ID of its OBU type so `check-feature-status` can track the gap.
- **Reject-before-write via scratch.** Draft the whole OBU payload into a scratch `BitWriter` and
  `append` on full success; a delegated sub-writer reject (sequence/metadata `NonCanonical*`) or a
  passthrough-length mismatch leaves the caller untouched.
- **No model change.** Purely additive.

## Testing

Round-trip via `dispatch_obu_payload` (or the per-type parser): for each *written* type
(temporal-delimiter, a sequence header from a fixture, padding with opaque bytes, metadata short +
group with passthrough), write the payload, reparse, and assert the reparsed `ParsedObu` equals the
original; byte-exact on the canonical subset. One `Unimplemented` test per a couple of unwritten
types (assert the typed error + `bit_len() == 0`). One reject-propagation test (a non-canonical
sequence/metadata model propagates with `bit_len() == 0`). A `write_complete_obu` round-trip through
`write_annexb_obu` framing for at least the sequence-header + metadata cases. A never-panics proptest
over arbitrary constructed `ParsedObu` is optional given the sub-writers are already fuzzed.
