# Tasks: HLS layer-configuration-record and atlas foundation

## 1. OpenSpec and matrix setup

- [x] Add this OpenSpec change and validate it with `openspec validate hls-lcr-atlas-foundation --strict`.
- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` rows `AV2-5.8-LAYER-CONFIG-RECORD`,
  its §5.8.1 through §5.8.9 child rows, `AV2-5.9-ATLAS-SEGMENT`, the new §5.9.1 through
  §5.9.5 child rows, and `AV2-7.3.8-HLS-AVAILABILITY`.

## 2. splot-core parsers

- [x] Add `BitReader::read_leb128()` with positive/EOF/overflow tests.
- [x] Add `headers::layer_config_record` parsing the full § 5.8 syntax into strong types.
- [x] Add `headers::atlas_segment` parsing the full § 5.9 syntax, range-checking the
  mode and segment/region counts.
- [x] Add `ParsedObu::LayerConfigurationRecord` / `ParsedObu::AtlasSegment` and dispatch
  them through `dispatch_obu_payload`, finishing the extensible-OBU tail.
- [x] Add `Error::InvalidLayerConfigRecord` / `Error::InvalidAtlasSegment` typed errors.

## 3. splot-validate checks

- [x] Add stateless `LayerConfigRecordSyntax` and `AtlasSegmentSyntax` checks and map
  the new core errors to `lcr/*` / `atlas/*` diagnostics.
- [x] Extend `HlsAvailabilityStore` with global-LCR, local-LCR, and local-atlas records.
- [x] Emit `lcr/global-lcr-unavailable`, `atlas/local-atlas-unavailable`,
  `lcr/global-xlayer-map-missing-xlayer`, and `hls/unavailable-layer-configuration-record`.
- [x] Allowlist the `lcr/` and `atlas/` diagnostic prefixes in
  `xtask/src/feature_status.rs` and `docs/FEATURE-TRACKING.md`.

## 4. Tests and proof

- [x] Core parser tests (`cargo test -p splot-core lcr`, `cargo test -p splot-core atlas`).
- [x] Validator tests (`cargo test -p splot-validate hls`, `lcr`, `atlas`).
- [x] `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [x] `cargo xtask check-feature-status`, `cargo xtask spec-coverage`, `cargo xtask ci`.
