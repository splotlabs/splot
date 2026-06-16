# Change: metadata-obu-writer

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.17-METADATA` (the metadata OBU writer; advances its `write` stage, with the typed
  payload children — § 5.17.1 through § 5.17.13)

## Why

Next `obu-payload-writers-*` family of the writer mission: the **metadata OBU** writers, the
inverse of the § 5.17 `metadata_short_obu()` / `metadata_group_obu()` parsers. With them, splot
can round-trip every parser-produced metadata OBU.

## What changes

- **Writers** (`crates/splot-core/src/write/metadata.rs`): `write_metadata_short_obu`,
  `write_metadata_group_obu`, `write_metadata_unit`, and a `write_metadata_payload` dispatch over
  the 11 `MetadataPayload` variants — each validating the model up front (reject-before-write;
  `bit_len() == 0` on every reject).
- **Fully-modeled payloads are byte-exact:** HDR CLL/MDCV, timecode, scan-type,
  temporal-point-info, decoded-frame-hash, and banding-hints carry every field, so the writer
  reproduces them exactly.
- **Length-summarized payloads use a passthrough.** `MetadataItutT35`, `MetadataIccProfile`,
  `MetadataUserDataUnregistered`, and `MetadataUnknownRaw` store only the payload **length** (the
  model deliberately summarizes unbounded payload bytes for the inspector). The writer accepts the
  opaque payload bytes as a separate input so it reproduces them byte-exactly **without a model
  change** — keeping the model's length-summary design. The round-trip tests slice those bytes
  from the original input.
- **`leb128` byte-exactness.** The short-OBU `metadata_type` carries its
  `metadata_type_leb128_bytes` in the model (and in the derived `PartialEq`), so the writer
  reproduces the **exact** byte count (honouring a non-minimal encoding) rather than
  canonicalizing — canonicalizing would break the *semantic* round-trip, not just byte-exactness.
  The group-unit `muh_payload_size` is likewise byte-exact: its leb128 length is derived from the
  stored `muh_header_size` (the parser's header-byte accounting forces it). The group OBU's
  `metadata_type` and `metadata_unit_cnt_minus_1` leb128 byte counts are *not* modeled (the parser
  discards them), so those are emitted minimal — byte-exact on the canonical (minimal-leb128)
  subset, semantic always (documented, like the § 5.4 sequence-header leb128 cases). The
  byte-granular unit padding (§ 6.16.1 "can take any value") and the discarded
  `muh_header_extension_byte`s are zero-filled — byte-exact on the zero-padded subset, semantic
  always.
- **No model field.** Adds one additive, writer-only `WriteError::NonCanonicalMetadata { what }`
  reject variant, consistent with the per-family `NonCanonicalSequenceValue` /
  `NonCanonicalFrameHeader` precedent (the parser/decoder error model is untouched).

## Validator impact

None. No new diagnostics.

## Non-goals

- No tile-group / film-grain payload writers (separate slices).
- No public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write` module).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (the `AV2-5.17-METADATA` umbrella + child rows) +
  regenerated
  `docs/FEATURE-STATUS.md`.
