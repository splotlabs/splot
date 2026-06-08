# Proposal: Metadata + padding OBU foundation

## Summary

Add typed, panic-free, length-bounded parsers for the AV2 `OBU_PADDING`
(`padding_obu()`, § 5.16) and the two metadata OBUs — `OBU_METADATA_SHORT`
(`metadata_short_obu()`, § 5.17.2) and `OBU_METADATA_GROUP` (`metadata_group_obu()`,
§ 5.17.3) — together with the shared `metadata_unit()` (§ 5.17.1) and its § 5.17.4-
§ 5.17.13 typed child payloads. Dispatch all three OBU types through
`open_bitstream_unit`, surface them in `inspect --json`, extend the validator with the
locally-decidable § 6.15 / § 6.16 diagnostics, and refine the temporal-unit ordering
state machine so global prefix vs. suffix metadata is classified from the parsed
`metadata_is_suffix` bit.

## Why

`OBU_PADDING` and the metadata OBUs are the last two non-HLS payload OBU families that
the dispatcher still reports as `unimplemented`. Padding carries the § 5.16 / § 6.15
"last non-zero byte is the trailing-bits byte" rule that the ordering state machine
already special-cases, and the metadata OBUs carry HDR, timecode, ITU-T T.35, ICC,
scan-type, banding, decoded-frame-hash, and user-data payloads that downstream tools
need to see. The temporal-unit ordering code (AV2 § 7.3.7) currently classifies *all*
global metadata as an HLS prefix because metadata is not parsed; § 6.16.3 makes that
correct only for `metadata_is_suffix == 0`.

## What changes

- `splot-core` gains `headers::padding` (`PaddingObu`, `parse_padding_obu`) and
  `headers::metadata` (the short/group/unit models, the § 5.17.4-§ 5.17.13 child
  payloads, `MetadataType`, and `parse_metadata_short` / `parse_metadata_group`).
- `bitio::BitReader` gains `take_bytes(n)`, which splits off a sub-reader bounded to
  exactly `n` bytes so `metadata_unit(metadataPayloadSize)` child syntax cannot
  overread its declared size.
- `error::Error` gains `InvalidPadding` (`PaddingErrorKind`) and `InvalidMetadata`
  (`MetadataErrorKind`) for the structural § 5.16 / § 5.17 violations that prevent
  further parsing.
- `open_bitstream_unit` dispatch grows `ParsedObu::Padding`,
  `ParsedObu::MetadataShort`, and `ParsedObu::MetadataGroup`, with `feature_id()` /
  `syntax_name()` entries; `inspect --json` gains `padding`, `metadata_short`, and
  `metadata_group` views that summarize raw payload lengths rather than dumping bytes.
- `splot-validate` adds stateless `padding/*` and `metadata/*` checks for the locally-
  decidable § 6.15 / § 6.16 rules, and refines `TemporalUnitState` so global
  prefix metadata participates in § 7.3.7 prefix ordering while global suffix metadata
  is not flagged as a prefix.

## Non-goals

The § 6.16 metadata *semantic* validation below is out of scope here (stateful or
decoder/frame-parsing-blocked) and is tracked by the `metadata-semantic-validation`
change:

- Metadata persistence / cancellation lifetime tracking across OBUs (§ 6.16.3).
- Decoded-frame-hash verification against decoded pixels (§ 6.16.13) — no decoder.
- Scan-type cross-check with content interpretation / CVS-wide `mps_pic_struct_type`
  consistency (§ 6.16.10).
- Detailed prefix/suffix placement of metadata *inside* coded frame units
  (§ 7.3.3 / § 7.3.4) — needs frame/tile parsing.

Also out of scope: frame header, tile payload, encoder/writer, and AVM differential
testing.

## Feature IDs

- `AV2-5.16-PADDING`
- `AV2-5.17-METADATA` (umbrella)
- `AV2-5.17.1-METADATA-UNIT`, `AV2-5.17.2-METADATA-SHORT`, `AV2-5.17.3-METADATA-GROUP`
- `AV2-5.17.4-METADATA-ITUT-T35` through `AV2-5.17.13-METADATA-USER-DATA-UNREGISTERED`

## Acceptance criteria

- `padding_obu()`, `metadata_short_obu()`, and `metadata_group_obu()` parse into typed
  `ParsedObu` variants, never panic on arbitrary input, never read past the OBU
  boundary, and bound every `metadata_unit()` to exactly `metadataPayloadSize` bytes.
- An all-zero non-empty padding payload is rejected; an empty padding payload and a
  one-byte trailing-only payload are accepted.
- The validator emits `padding/all-zero-payload`, `padding/invalid-trailing-bits`,
  `metadata/unit-payload-underflow`, `metadata/short-layer-idc-out-of-range`,
  `metadata/group-unit-count-too-large`, `metadata/group-header-underflow`,
  `metadata/group-reserved-bits-nonzero`, `metadata/group-xlayer-map-global-bit-set`,
  `metadata/group-mlayer-map-below-obu-mlayer`, `metadata/temporal-point-info-not-short`,
  `metadata/timecode-seconds-out-of-range`, `metadata/timecode-minutes-out-of-range`,
  `metadata/timecode-hours-out-of-range`, and `metadata/scan-type-pic-struct-reserved`
  for the corresponding violations.
- Global prefix metadata (`metadata_is_suffix == 0`) participates in temporal-unit
  prefix ordering; global suffix metadata (`metadata_is_suffix == 1`) is not flagged as
  a global prefix after coded-layer OBUs.
- `cargo xtask ci` and `openspec validate metadata-padding-foundation --strict` pass.
