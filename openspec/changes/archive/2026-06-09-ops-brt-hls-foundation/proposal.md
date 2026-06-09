# Proposal: Operating point set + buffer removal timing HLS foundation

## Summary

Add typed, panic-free parsers for the AV2 `OBU_OPERATING_POINT_SET`
(`operating_point_set_obu()`, § 5.10, with its `operating_point_payload()` and
§ 5.11.1-§ 5.11.5 children) and `OBU_BUFFER_REMOVAL_TIMING`
(`buffer_removal_timing_obu()`, § 5.12) payloads, dispatch them through
`open_bitstream_unit`, surface them in `inspect --json`, and extend the validator with
non-monotonic active-OPS state so it can check the locally-decidable § 6.10 OPS
conformance rules and the § 6.11 / § 7.3.8.5 buffer-removal-timing references.

## Why

The validator already models sequence-header, multi-frame-header, layer-configuration-
record, and local atlas-segment availability. `OBU_OPERATING_POINT_SET` is the next
high-level-syntax control-plane object, and `OBU_BUFFER_REMOVAL_TIMING` is its first
consumer: an OPS-dependent BRT references an OPS by `br_ops_id` and must carry a
`br_ops_cnt` equal to that OPS's `ops_cnt` (§ 6.11). Without parsing and tracking OPS
state the inspector shows both OBUs as `unimplemented` and the validator cannot reason
about operating-point metadata or buffer-removal-timing references.

## What changes

- `splot-core` gains two parser modules, `headers::operating_point_set` and
  `headers::buffer_removal_timing`, that read the full § 5.10-§ 5.12 syntax (no skipped
  bits) into strong types, retaining the values the validator needs (reserved bits, the
  global `ops_mlayer_info_idc`, each PTL reserved field, declared `ops_data_size`
  alongside the computed `opsBytes`, and inherited-operating-point references).
- `open_bitstream_unit` dispatch grows `ParsedObu::OperatingPointSet` (extensible OBU
  tail) and `ParsedObu::BufferRemovalTiming` (non-extensible, `trailing_bits()` only),
  and `inspect --json` gains `operating_point_set` and `buffer_removal_timing` views.
- `splot-validate` adds a non-monotonic active-OPS store with § 6.10.1 reset/update
  semantics, emits the locally-decidable `ops/*` diagnostics, and validates `brt/*`
  OPS references under external-HLS-disabled mode, replacing the temporal-unit
  ordering TODO for `OBU_BUFFER_REMOVAL_TIMING` with a spec-backed, tested rule.

## Non-goals

- Annex A.4 level conformance derived from OPS PTL fields.
- Annex E smoothing-buffer / decoder schedule / resource validation.
- OPS dependency-map agreement with the activated sequence header
  (`MLayerDependencyMap` / `TLayerDependencyMap`, § 6.10.7) — not exposed by the
  sequence-header model.
- A hard ordering error for a global `OBU_BUFFER_REMOVAL_TIMING`
  (`brt/global-ordering-position`) — § 7.3.7 does not list BRT among the global
  temporal-unit prefix OBUs; modeling it needs decoder-model / random-access state.
- Full frame header, tile payload, encoder/writer, quantization matrix, film grain,
  metadata payloads, and AVM differential testing.

## Feature IDs

- `AV2-5.10-OPERATING-POINT-SET`, `AV2-5.10-OPS-SYNTAX-ELEMENTS`
- `AV2-5.11-OPERATING-POINT-PAYLOAD`
- `AV2-5.11.1-OPS-AGGREGATE-INFO` through `AV2-5.11.5-OPS-MLAYER-INFO`
- `AV2-5.12-BUFFER-REMOVAL-TIMING`

## Acceptance criteria

- `operating_point_set_obu()` and `buffer_removal_timing_obu()` parse into typed
  `ParsedObu` variants, never panic on arbitrary input, and never read past the payload
  boundary.
- The validator emits `ops/local-reserved-bits-nonzero`,
  `ops/mlayer-info-idc-reserved`, `ops/ptl-reserved-bits-nonzero`,
  `ops/payload-size-mismatch`, `ops/inherited-op-index-out-of-range`,
  `brt/unavailable-operating-point-set`, and `brt/ops-count-mismatch` for the
  corresponding violations, and does not emit a hard missing-OPS error when external
  HLS is provided.
- `cargo xtask ci` and `openspec validate ops-brt-hls-foundation --strict` pass.
