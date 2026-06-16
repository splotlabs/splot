# Design: metadata-obu-writer

## Context

The § 5.17 metadata OBUs come in two forms: `metadata_short_obu()` (§5.17.2,
`MetadataShortObu`) and `metadata_group_obu()` (§5.17.3, `MetadataGroupObu` with N
`MetadataGroupUnit`s). Both carry a `muh_*` header (with a cancel-flag early-out) and a
`metadata_unit()` (§5.17.1, `MetadataUnit`) whose `metadata_type` `leb128()` selects one of 11
typed `MetadataPayload` variants. Several payload structs (`MetadataItutT35`,
`MetadataIccProfile`, `MetadataUserDataUnregistered`, `MetadataUnknownRaw`) store only the payload
**length** — the parser deliberately summarizes unbounded payload bytes for the inspector — while
the rest are fully modeled.

## Decisions

- **Additive — passthrough for the length-summarized payloads, not a model extension.** The
  mission invariants define the target: semantic round-trip always, byte-exact where achievable.
  For the length-summarized payloads the model does not carry the bytes, and they are *unbounded*
  (ICC profiles, user data), so storing them in the model would bloat it against its deliberate
  length-summary design (unlike the small bounded #4g CCSO offsets that warranted a model
  extension). Instead the writer takes the opaque payload bytes as a separate input and emits them
  verbatim — byte-exact with no model change. The round-trip tests slice those bytes from the
  original input.
- **`leb128` byte-exactness, not blind canonicalization.** The short-OBU `metadata_type` stores
  its `metadata_type_leb128_bytes` *in the model* (so it is in the derived `PartialEq`); emitting a
  minimal encoding for a model parsed from a non-minimal stream would make
  `parse(write(x)) != x` — a *semantic* round-trip break, not merely a byte-exactness one. So the
  writer reproduces the exact byte count via a local `write_leb128_with_len` helper (reject when
  the stored count is `0`, `> 8`, or below the value's minimal length). The group-unit
  `muh_payload_size` leb128 length is *not* stored directly, but the parser's header accounting
  forces it: `payload_size_bytes = muh_header_size - 2 - layer_map_bytes - header_extension_len`,
  so the writer derives that exact length and reproduces `muh_payload_size` byte-exactly too. Only
  the group `metadata_type` / `metadata_unit_cnt_minus_1` leb128 (counts the parser discards) are
  emitted minimal — semantic always, byte-exact on the minimal subset.
- **Reject-before-write for the muh/cancel gating, via the scratch-writer pattern.** Each composing
  writer drafts the whole OBU/unit into a local `BitWriter` and `append`s it to the caller only on
  full success, so a mid-composition reject never leaves a partial buffer. Every conditional the
  parser reads (the `muh_cancel_flag` early-out and the fields it gates; the `muh_header_size`
  accounting; the `muh_xlayer_map`/`muh_mlayer_maps` presence, keyed off the OBU's `obu_xlayer_id`
  scope input as the parser is; the per-type payload domains; the `metadata_type` ↔ payload-variant
  agreement; the modeled `payload_size` vs `muh_payload_size`) is re-derived and a disagreeing
  model is rejected before any bit.
- **Byte-granular unit padding + extension bytes are zero-filled.** The `metadata_unit()` is bounded
  to `payload_size` bytes; the writer drafts the typed payload, rejects if it overflows
  `payload_size`, then zero-pads to the boundary (§ 6.16.1 padding "can take any value"). The
  discarded `muh_header_extension_byte`s are likewise zero-filled. Both are semantic-always,
  byte-exact on the zero-padded subset.
- **No panic on constructed models.** Every `f(n)` value is domain-checked before the write; the
  passthrough length is validated against the modeled `payload_len`; the `muh_header_size` /
  `payload_size_bytes` arithmetic is guarded (checked subtraction, leb-length bounds) so no
  underflow/over-wide write can panic.

## Testing

Round-trip via the public parsers (`parse_metadata_short` / `parse_metadata_group`) across every
payload type, both OBU forms, and the cancel-flag arms — byte-exact on the canonical (minimal-
leb128) subset with the passthrough bytes, and reparse-equal everywhere. One reject test per
reject path (asserting `bit_len() == 0`), a parser-driven round-trip property test, and a
never-panics-on-constructed-models proptest.
