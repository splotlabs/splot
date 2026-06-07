# Proposal: HLS layer-configuration-record and atlas foundation

## Summary

Add typed, panic-free parsers for the AV2 `OBU_LAYER_CONFIGURATION_RECORD`
(`layer_config_record_obu()`, § 5.8) and `OBU_ATLAS_SEGMENT`
(`atlas_segment_info_obu()`, § 5.9) payloads, dispatch them through
`open_bitstream_unit`, and extend the validator's HLS availability state so it can
check layer-configuration-record and local atlas-segment references (§ 7.3.8.3 /
§ 7.3.8.4) and a sequence header's `seq_lcr_id` resolution (§ 6.4.1).

## Why

The validator already models sequence-header and multi-frame-header availability
(§ 7.3.8.6 / § 7.3.8.7). LCR and atlas OBUs are the next high-level-syntax objects
referenced by other OBUs: a sequence header points at an LCR via `seq_lcr_id`, a
local LCR points at a global LCR and at a local atlas. Without parsing and tracking
these, the inspector shows them as `unimplemented` and the validator cannot diagnose
the layer/atlas reference rules in § 7.3.8.

## What changes

- `splot-core` gains a `BitReader::read_leb128()` descriptor and two new parser
  modules, `headers::layer_config_record` and `headers::atlas_segment`, that read the
  full § 5.8 / § 5.9 syntax (no skipped bits) into strong types.
- `open_bitstream_unit` dispatch grows `ParsedObu::LayerConfigurationRecord` and
  `ParsedObu::AtlasSegment`, finished with the shared extensible-OBU tail logic.
- `splot-validate` gains stateless `lcr/*` and `atlas/*` syntax checks and extends the
  HLS availability store with global-LCR, local-LCR, and local-atlas records, emitting
  the § 7.3.8.3 / § 7.3.8.4 / § 6.4.1 reference diagnostics.

## Non-goals

- MFH layer-dependency-map checks (`MLayerDependencyMap` / `TLayerDependencyMap` are
  not exposed by the sequence-header model).
- A "repeated LCR/atlas must be identical" check (§ 6.8 / § 6.9 define no such
  requirement).
- A hard error for a missing global atlas (§ 7.3.8.4 says it "can be available").
- Bitstream writer, encoder, decoder reconstruction, and AVM differential testing.

## Feature IDs

- `AV2-5.8-LAYER-CONFIG-RECORD` and its §5.8.1 through §5.8.9 child rows
- `AV2-5.9-ATLAS-SEGMENT` and its new §5.9.1 through §5.9.5 child rows
- `AV2-7.3.8-HLS-AVAILABILITY`

## Acceptance criteria

- `layer_config_record_obu()` and `atlas_segment_info_obu()` parse into typed
  `ParsedObu` variants, never panic, and never read past the payload boundary.
- The validator emits `lcr/reserved-bits-nonzero`, `lcr/payload-size-overflow`,
  `lcr/global-lcr-unavailable`, `lcr/global-xlayer-map-missing-xlayer`,
  `atlas/segment-mode-out-of-range`, `atlas/region-dimension-out-of-range`,
  `atlas/segment-count-out-of-range`, `atlas/local-atlas-unavailable`, and
  `hls/unavailable-layer-configuration-record` for the corresponding violations.
- `cargo xtask ci` and `openspec validate hls-lcr-atlas-foundation --strict` pass.
