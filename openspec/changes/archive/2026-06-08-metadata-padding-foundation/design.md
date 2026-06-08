# Design: Metadata + padding OBU foundation

## Padding parser

`padding_obu()` (§ 5.16) has no coded length: `obu_padding_length` is derived. The
§ 5.16 / § 6.15 rule is that the last byte of valid content is the last non-zero byte,
and `trailing_bits()` runs from there to the end of the payload. `parse_padding_obu`
implements this directly on the OBU payload slice:

- An empty payload (`obuPayloadSize == 0`) is legal: `padding_len == 0`,
  `trailing_len == 0`.
- Otherwise it finds the last non-zero byte `L`. If there is none, the payload is all
  zero, which § 5.16 forbids (`padding/all-zero-payload`).
- Bytes `[0, L)` are `obu_padding_byte` values (arbitrary). The slice `[L, end)` is
  parsed as `trailing_bits((end - L) * 8)`; a malformed pattern is
  `padding/invalid-trailing-bits`.

Because the padding parser consumes the entire payload (padding bytes plus its own
trailing bits), `open_bitstream_unit` dispatch does **not** call the shared
`finish_obu_payload()` for `OBU_PADDING` — that would double-consume the trailing bits.

## Metadata parsers and bounded `metadata_unit`

`metadata_short_obu(obuPayloadSize)` (§ 5.17.2) reads the 1-byte header
(`metadata_is_suffix`, `muh_layer_idc`, `muh_cancel_flag`, `muh_persistence_idc`) and
`metadata_type` (`leb128()`, retaining `Leb128Bytes`). On `muh_cancel_flag` it returns
immediately, leaving the reader positioned for the OBU's `trailing_bits()`. Otherwise it
computes `metadataPayloadSize = obuPayloadSize - 2 - Leb128Bytes` with checked
arithmetic (underflow is `metadata/unit-payload-underflow`) and parses
`metadata_unit(metadataPayloadSize)`.

`metadata_group_obu()` (§ 5.17.3) reads `metadata_is_suffix`,
`metadata_necessity_idc`, `metadata_application_id`, and `metadata_unit_cnt_minus_1`
(`< 16383`, else `metadata/group-unit-count-too-large`), then for each unit reads
`metadata_type`, `muh_header_size`, `muh_cancel_flag`, and — for a non-cancelled unit —
`muh_payload_size`, the layer/persistence/priority/reserved fields, and the optional
`muh_xlayer_map` / `muh_mlayer_map` maps, decrementing `headerRemainingBytes` with
checked arithmetic (underflow is `metadata/group-header-underflow`). The remaining
header-extension bytes are consumed and ignored, then `metadata_unit(muh_payload_size)`
is parsed for a non-cancelled unit.

`metadata_unit(metadataPayloadSize)` (§ 5.17.1) is bounded with
`BitReader::take_bytes(metadataPayloadSize)`: the parent reader advances past exactly
`metadataPayloadSize` bytes, and child syntax runs on the returned sub-reader, so a
child that reads past the declared size hits the sub-reader's end and the parser maps
that EOF to `metadata/unit-payload-underflow`. The `metadata_unit_remaining_bit` bits
(§ 6.16.1, "can take any value") are not validated — the parent already advanced past
them. Reserved / unknown `metadata_type` values are preserved as `UnknownRaw` (raw
length only), never `Unimplemented`. Variable-length payloads (ITU-T T.35, ICC,
user-data) summarize their byte length rather than retaining the blob, so the inspector
never dumps unbounded raw bytes.

`take_bytes` requires byte alignment, which every AV2 length-bounded payload site
guarantees structurally (the metadata header and per-unit fields are whole bytes), so
the precondition holds for all inputs and is asserted in debug builds.

## Dispatch and inspector

`open_bitstream_unit` dispatch gains `ParsedObu::Padding`, `ParsedObu::MetadataShort`,
and `ParsedObu::MetadataGroup`. Metadata OBUs are not extensible
(`ObuType::is_extensible_obu` is `false`), so they finish with `trailing_bits()` via the
shared `finish_obu_payload()`; padding finishes inside its own parser. `inspect --json`
gains `padding`, `metadata_short`, and `metadata_group` views: the metadata views report
the header fields, each unit's `metadata_type` (value + name) and `payload_size`, and
raw-payload *lengths* only.

## Validator diagnostics

Stateless `padding/syntax` and `metadata/syntax` checks (alongside the existing
per-OBU syntax checks) parse the payload, surface structural § 5.16 / § 5.17 errors via
`syntax_error_diagnostic`, validate the metadata payload tail, and emit the locally-
decidable § 6.15 / § 6.16 diagnostics:

- `padding/all-zero-payload` (§ 5.16), `padding/invalid-trailing-bits` (§ 5.16) — the
  "at least one non-zero byte" rule is stated in the § 5.16 NOTE, not § 6.15
- `metadata/unit-payload-underflow` (§ 6.16.1), `metadata/group-unit-count-too-large`
  (§ 6.16.3), `metadata/group-header-underflow` (§ 6.16.3)
- `metadata/short-layer-idc-out-of-range` (§ 6.16.2)
- `metadata/group-reserved-bits-nonzero` (§ 6.16.3) — a **warning**, matching the
  `content-interpretation/reserved-bits-nonzero` precedent: § 6.16.3 says
  `muh_reserved_zero_2bits` "must be set to zero and shall be ignored by decoders", so a
  non-zero value is a producer anomaly, not a decode-breaking error.
- `metadata/group-xlayer-map-global-bit-set` (§ 6.16.3),
  `metadata/group-mlayer-map-below-obu-mlayer` (§ 6.16.3)
- `metadata/temporal-point-info-not-short` (§ 6.16.11)
- `metadata/timecode-seconds-out-of-range`, `metadata/timecode-minutes-out-of-range`,
  `metadata/timecode-hours-out-of-range` (§ 6.16.7)
- `metadata/scan-type-pic-struct-reserved` (§ 6.16.10)

## Ordering

§ 7.3.7 lists the global temporal-unit prefix OBUs exhaustively, including *global
prefix metadata*. § 6.16.3 defines `metadata_is_suffix`: a prefix (`== 0`) appears
before frame data, a suffix (`== 1`) after. `TemporalUnitState` reads the first payload
bit of a metadata OBU and classifies it:

- global, `metadata_is_suffix == 0` → global HLS prefix (flagged if it follows a coded
  extended layer unit, `obu-order/global-hls-after-coded-layer`);
- global, `metadata_is_suffix == 1` → not a prefix; left unclassified (never flagged as
  a global prefix);
- non-global → coded extended layer OBU (participates in ascending `obu_xlayer_id`
  order), as before.

A metadata OBU whose first bit cannot be read is left unclassified (sound-over-complete:
the structural parse error is reported elsewhere). Detailed placement of suffix metadata
*inside* coded frame units (§ 7.3.3 / § 7.3.4) stays deferred until frame/tile parsing
exists.

## Boundaries

Parser coverage for padding and the metadata OBUs is complete, but metadata semantic
validation stays `partial`: persistence/cancellation lifetime tracking, decoded-frame-
hash verification, scan-type CVS-wide consistency, and frame-unit suffix/prefix
placement are not implemented, so the umbrella `AV2-5.17-METADATA` keeps
`validate = "partial"`. Those deferred items are tracked by the
`metadata-semantic-validation` change.
