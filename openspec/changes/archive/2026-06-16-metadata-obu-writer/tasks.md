# Tasks

## Writers (additive — no model change)
- [x] `write/metadata.rs`: `write_metadata_short_obu` (§5.17.2), `write_metadata_group_obu`
      (§5.17.3), `write_metadata_unit` (§5.17.1), and `write_metadata_payload` over the 11
      `MetadataPayload` variants, each with an up-front `check_*_encodable` (reject-before-write,
      `bit_len() == 0`).
- [x] Fully-modeled payloads (HDR CLL/MDCV, timecode, scan-type, temporal-point-info,
      decoded-frame-hash, banding-hints) byte-exact; length-summarized payloads (ITU-T T.35, ICC
      profile, user-data-unregistered, unknown-raw) reproduced via an opaque payload-bytes
      passthrough input. Register + re-export in `write/mod.rs`. No model / `WriteError` change.
- [x] Honour the `muh_*` header gating (cancel-flag arms, the group `muh_header_size` bound and
      its conditional fields, the layer-map presence) and the `leb128`-minimal canonicalization of
      `metadata_type`; reject any model the parser could not have produced.

## Tests and proof
- [x] Round-trip tests across every payload type + both OBU forms + the cancel-flag arms (parse a
      hand-built or fixture metadata OBU, write it back with the same passthrough bytes, assert
      byte-exact on the canonical subset + reparse-equal); one reject test per reject path
      (`bit_len() == 0`). A parser-driven round-trip property test + a never-panics-on-constructed
      proptest.

## Matrix and docs
- [x] Advance `write` on `AV2-5.17-METADATA` + the § 5.17.1 through § 5.17.13 child rows it covers
      (note the passthrough / leb128-canonical subset). Regenerate `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate metadata-obu-writer --strict`
