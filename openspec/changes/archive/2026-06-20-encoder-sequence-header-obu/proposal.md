## Why

The gating piece for a decodable temporal unit. The decoder's minimal-tier IVF frame
requires its OBUs in order `OBU_TEMPORAL_DELIMITER`, `OBU_SEQUENCE_HEADER`, then the frame
OBU. The temporal delimiter (brick 6) and the frame OBU (brick 5) are done. This adds the
sequence header — the activated tier description the frame header parses against.

## What Changes

- Add `ENC-SEQUENCE-HEADER-OBU` as an encoder writer-input-bridge feature (code in
  `splot-core`, driven by the encoder mission).
- Add `build_minimal_intra_sequence_header() -> Result<SequenceHeader,
  MinimalIntraSequenceHeaderError>`: parse-backs the committed `syn-cos-intra-64x64-q180`
  conformance vector's 11-byte `sequence_header()` body (the decoder's minimal-tier sequence
  header) into a `SequenceHeader` — conformant by construction.
- Add `encode_minimal_intra_sequence_header_obu() -> Result<Vec<u8>, ...>`: the
  body-plus-§5.2.3-tail payload (via `write_obu_payload`, not the body-only
  `write_sequence_header`) under the no-extension `OBU_SEQUENCE_HEADER` § 5.2.2 header, in
  Annex B framing — byte-exact to the conformance vector's sequence-header OBU.
- Add the `MinimalIntraSequenceHeaderError` typed error (`Parse` / `Write` arms).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder minimal-intra sequence-header OBU.

## Impact

- Affected code: `crates/splot-core/src/headers/frame/encoder_input.rs` (the const, two
  functions, the typed error, tests), `crates/splot-core/src/headers/frame/mod.rs`
  (re-exports).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: two new public functions and one new public error, `splot-core`. No
  dependency-graph change.
- Validator/CLI impact: none.
