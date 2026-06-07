# validator spec delta

## ADDED Requirements

### Requirement: Layer configuration record and atlas syntax checks

`splot-validate` SHALL run stateless syntax checks over `OBU_LAYER_CONFIGURATION_RECORD`
and `OBU_ATLAS_SEGMENT` payloads, surfacing parse and range violations as `lcr/*` and
`atlas/*` diagnostics and warning on a non-zero reserved-zero field (AV2 § 6.8).

#### Scenario: non-zero reserved field

- **GIVEN** a layer configuration record whose `lcr_global_reserved_zero_5bits` is
  non-zero
- **WHEN** the validator runs
- **THEN** it SHALL emit a `lcr/reserved-bits-nonzero` warning.

#### Scenario: out-of-range atlas mode

- **GIVEN** an atlas segment OBU with `ats_atlas_segment_mode_idc` greater than 4
- **WHEN** the validator runs
- **THEN** it SHALL emit an `atlas/segment-mode-out-of-range` error.

### Requirement: Layer configuration record availability

`splot-validate` SHALL track in-band layer-configuration-record and local
atlas-segment availability and emit diagnostics when a reference cannot be resolved
(AV2 § 7.3.8.3 / § 7.3.8.4), gating the hard errors on external HLS being disabled.

#### Scenario: local LCR references an unavailable global LCR

- **GIVEN** a local LCR whose `lcr_global_id` is non-zero and no preceding global LCR
  has that `lcr_global_config_record_id`
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit a `lcr/global-lcr-unavailable` error.

#### Scenario: local LCR references an available local atlas

- **GIVEN** a local atlas segment OBU precedes a local LCR that references it via
  `lcr_local_atlas_id` in the same extended layer
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit `atlas/local-atlas-unavailable`.

#### Scenario: local LCR references an unavailable local atlas

- **GIVEN** a local LCR whose `lcr_local_atlas_id` has no preceding local atlas segment
  OBU in the same extended layer
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit an `atlas/local-atlas-unavailable` error.

### Requirement: Sequence header `seq_lcr_id` resolution

`splot-validate` SHALL resolve a sequence header's `seq_lcr_id` (when non-zero) to an
available local LCR (same xlayer) or, failing that, an available global LCR whose
`lcr_xlayer_map` includes the sequence header's xlayer (AV2 § 6.4.1 / § 7.3.8.6).

#### Scenario: seq_lcr_id resolves to a local or global LCR

- **GIVEN** a preceding LCR (local with matching `lcr_local_id`, or global with
  matching `lcr_global_config_record_id` whose `lcr_xlayer_map` includes the header's
  xlayer)
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit `hls/unavailable-layer-configuration-record` or
  `lcr/global-xlayer-map-missing-xlayer`.

#### Scenario: seq_lcr_id resolves to no LCR

- **GIVEN** a sequence header with `seq_lcr_id != 0` and no matching LCR
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit a `hls/unavailable-layer-configuration-record` error.

#### Scenario: global LCR omits the header's xlayer

- **GIVEN** a sequence header whose `seq_lcr_id` resolves to a global LCR whose
  `lcr_xlayer_map` does not include the header's `obu_xlayer_id`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `lcr/global-xlayer-map-missing-xlayer` error.

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
