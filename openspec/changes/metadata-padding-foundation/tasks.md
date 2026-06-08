# Tasks: Metadata + padding OBU foundation

## 1. Planning and tracking

- [x] Read `AGENTS.md`, `docs/FEATURE-TRACKING.md`, and `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Read the AV2 v1.0.0 spec mirror for § 5.2.1, § 5.2.3, § 5.16, § 5.17.*, § 6.15,
      § 6.16.*, and § 7.3.7.
- [x] Create this OpenSpec change.
- [x] Update matrix rows to use `openspec_change = "metadata-padding-foundation"`.
- [x] Keep statuses honest: parser stages become `done`; the metadata umbrella stays
      `validate = "partial"`.

## 2. Core padding parser (`splot-core`)

- [x] Add `Error::InvalidPadding` + `PaddingErrorKind` (all-zero, invalid trailing).
- [x] Add `headers::padding` (`PaddingObu`, `parse_padding_obu`) implementing the
      § 5.16 / § 6.15 last-non-zero-byte rule.
- [x] Unit-test empty payload, one-byte trailing-only, arbitrary padding bytes,
      all-zero rejection, malformed trailing bits, and the never-panic property.

## 3. Core metadata parser (`splot-core`)

- [x] Add `BitReader::take_bytes(n)` for bounded `metadata_unit` sub-readers + tests.
- [x] Add `Error::InvalidMetadata` + `MetadataErrorKind` (unit underflow, group unit
      count too large, group header underflow).
- [x] Add `headers::metadata` models: `MetadataType`, `MetadataShortObu`,
      `MetadataGroupObu`, `MetadataGroupUnit`, `MetadataUnit`, `MetadataPayload`, and the
      § 5.17.4-§ 5.17.13 child structs (HDR CLL/MDCV, ITU-T T.35, timecode, decoded
      frame hash, banding hints, ICC, scan type, temporal point info, user data, raw).
- [x] Parse § 5.17.2 short and § 5.17.3 group with checked `metadataPayloadSize` /
      `headerRemainingBytes`, bounded `metadata_unit`, and `UnknownRaw` preservation.
- [x] Unit-test every child type, payload-size underflow, EOF at variable/fixed-width
      boundaries, unknown-type raw preservation, group header underflow, layer maps,
      cancellation, and the never-panic property.

## 4. Dispatch and inspector

- [x] Dispatch `OBU_PADDING` (parser owns its trailing bits), `OBU_METADATA_SHORT`, and
      `OBU_METADATA_GROUP` (non-extensible `trailing_bits()` tail) through
      `dispatch_obu_payload`; remove them from the unimplemented branch.
- [x] Add `ParsedObu` variants with `feature_id()` / `syntax_name()`.
- [x] Surface `padding`, `metadata_short`, and `metadata_group` in `inspect --json`,
      summarizing raw payload lengths (never dumping bytes).
- [x] Add committed fixtures and CLI inspector tests.

## 5. Validator diagnostics and ordering (`splot-validate`)

- [x] Map `InvalidPadding` / `InvalidMetadata` in `syntax_error_diagnostic` and
      `error_location`.
- [x] Add stateless `padding/syntax` and `metadata/syntax` checks emitting the
      locally-decidable § 6.15 / § 6.16 diagnostics.
- [x] Refine `TemporalUnitState` so global metadata is classified by `metadata_is_suffix`.
- [x] Add validator tests for every new diagnostic id and the three ordering cases.
- [x] Add `padding/` and `metadata/` to the xtask diagnostic-prefix allowlist and
      `docs/FEATURE-TRACKING.md` § 12.

## 6. Proof

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test -p splot-core padding` / `metadata`
- [x] `cargo test -p splot-validate padding` / `metadata` / `temporal_unit`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`
- [x] `cargo xtask check-feature-status` / `spec-coverage` / `ci`
- [x] `openspec validate metadata-padding-foundation --strict`
- [x] Record exact command results in the PR notes.

## 7. Deferred (tracked, not done)

- [ ] Metadata persistence / cancellation lifetime store (§ 6.16.3).
- [ ] Decoded-frame-hash verification against decoded pixels (§ 6.16.13).
- [ ] Scan-type cross-check with content interpretation / CVS-wide consistency (§ 6.16.10).
- [ ] Full prefix/suffix placement of metadata inside coded frame units (§ 7.3.3 / § 7.3.4).
