# encoder-tools delta: metadata-obu-writer

## ADDED Requirements

### Requirement: metadata OBU writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.17
`metadata_short_obu()` and `metadata_group_obu()` parsers, including `metadata_unit()` and the 11
typed `metadata_*` payloads. For every model the writer accepts, reparsing the written bytes with
the corresponding parser SHALL yield the original on every structural field
(`parse(write(x)) == x`). The writers SHALL be additive (no model or parser-error change) and
SHALL never panic: a model the parser could not have produced SHALL be rejected with a typed
writer error before any bit is written.

The fully-modeled payloads SHALL be byte-exact. For the length-summarized payloads (ITU-T T.35,
ICC profile, user-data-unregistered, and the reserved/unknown raw payload), which the model
carries by length only, the writer SHALL accept the opaque payload bytes as a separate input and
emit them verbatim — byte-exact without a model change. The short-OBU `metadata_type` leb128
SHALL be reproduced byte-exactly from its modeled `metadata_type_leb128_bytes`, and the group-unit
`muh_payload_size` leb128 length SHALL be derived from the modeled `muh_header_size` so it too is
byte-exact. The group OBU's `metadata_type` and `metadata_unit_cnt_minus_1` leb128 byte counts are
not modeled and the byte-granular unit padding / discarded header-extension bytes are not carried,
so for those the round-trip SHALL be semantic universally and byte-exact on the canonical
(minimal-`leb128`, zero-padded) subset. A model the parser could not have produced SHALL be
rejected with a typed writer error before any bit (a single additive, writer-only
`NonCanonicalMetadata` reject variant; the parser/decoder error model is untouched).

#### Scenario: each metadata OBU round-trips

- **WHEN** a parsed `metadata_short_obu()` or `metadata_group_obu()` is written with the same
  passthrough payload bytes and reparsed
- **THEN** the reparsed structure SHALL equal the original, across every payload type, both OBU
  forms, and the `muh_cancel_flag` arms; and the bytes SHALL be byte-exact on the canonical subset.

#### Scenario: a non-reproducible metadata model is rejected before any bit

- **WHEN** a model carries a field outside its descriptor domain, a passthrough length that
  disagrees with the modeled `payload_len`, or a `muh_*` gated field that disagrees with its
  cancel-flag / header-size derivation
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
