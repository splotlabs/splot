# validator spec delta

## ADDED Requirements

### Requirement: Padding OBU diagnostics

`splot-validate` SHALL emit `padding/*` diagnostics for the locally-decidable AV2 v1.0.0
§ 5.16 / § 6.15 violations of `padding_obu()`.

#### Scenario: all-zero padding payload

- **GIVEN** a non-empty `OBU_PADDING` payload whose bytes are all zero
- **WHEN** the validator runs
- **THEN** it SHALL emit a `padding/all-zero-payload` error.

#### Scenario: malformed padding trailing bits

- **GIVEN** an `OBU_PADDING` whose last non-zero byte is not a valid `trailing_bits()`
  pattern
- **WHEN** the validator runs
- **THEN** it SHALL emit a `padding/invalid-trailing-bits` error.

#### Scenario: empty padding accepted

- **GIVEN** an `OBU_PADDING` with `obuPayloadSize == 0`
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `padding/*` error.

### Requirement: Metadata OBU diagnostics

`splot-validate` SHALL emit `metadata/*` diagnostics for the locally-decidable AV2 v1.0.0
§ 6.16 violations of the metadata OBUs.

#### Scenario: short layer idc out of range

- **GIVEN** a `metadata_short_obu()` with `muh_layer_idc >= 3`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/short-layer-idc-out-of-range` error (§ 6.16.2).

#### Scenario: group reserved bits non-zero

- **GIVEN** a non-cancelled group unit with `muh_reserved_zero_2bits != 0`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/group-reserved-bits-nonzero` warning (§ 6.16.3 says
  the field is ignored by decoders, so a non-zero value is a producer anomaly).

#### Scenario: group xlayer map global bit set

- **GIVEN** a global group unit with `muh_layer_idc == LAYER_VALUES` whose
  `muh_xlayer_map` has bit 31 set
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/group-xlayer-map-global-bit-set` error (§ 6.16.3).

#### Scenario: group mlayer map below obu mlayer

- **GIVEN** a group unit with `muh_layer_idc == LAYER_VALUES` whose `muh_mlayer_map` sets
  a bit `m` less than `obu_mlayer_id`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/group-mlayer-map-below-obu-mlayer` error (§ 6.16.3).

#### Scenario: temporal point info in a group

- **GIVEN** a `metadata_group_obu()` unit whose `metadata_type ==
  METADATA_TYPE_TEMPORAL_POINT_INFO`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/temporal-point-info-not-short` error (§ 6.16.11).

#### Scenario: timecode fields out of range

- **GIVEN** a `metadata_timecode()` with `seconds_value > 59`, `minutes_value > 59`, or
  `hours_value > 23` (when present)
- **WHEN** the validator runs
- **THEN** it SHALL emit the corresponding `metadata/timecode-seconds-out-of-range`,
  `metadata/timecode-minutes-out-of-range`, or `metadata/timecode-hours-out-of-range`
  error (§ 6.16.7).

#### Scenario: scan-type reserved pic struct

- **GIVEN** a `metadata_scan_type()` with `mps_pic_struct_type > 12`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/scan-type-pic-struct-reserved` error (§ 6.16.10).

### Requirement: Metadata temporal-unit ordering classification

`splot-validate` SHALL classify a metadata OBU for temporal-unit ordering (AV2 v1.0.0
§ 7.3.7) from its parsed `metadata_is_suffix` bit: global prefix metadata is a global
temporal-unit prefix OBU, global suffix metadata is not, and non-global metadata is a
coded extended layer OBU.

#### Scenario: global prefix metadata after a coded layer

- **GIVEN** a global metadata OBU with `metadata_is_suffix == 0` that follows a coded
  extended layer unit within a temporal unit
- **WHEN** the validator runs
- **THEN** it SHALL emit an `obu-order/global-hls-after-coded-layer` error.

#### Scenario: global suffix metadata after a coded layer

- **GIVEN** a global metadata OBU with `metadata_is_suffix == 1` that follows a coded
  extended layer unit within a temporal unit
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit an `obu-order/global-hls-after-coded-layer` error for it.

#### Scenario: non-global metadata uses coded xlayer order

- **GIVEN** a non-global metadata OBU within a temporal unit
- **WHEN** the validator runs
- **THEN** it SHALL treat it as a coded extended layer OBU for ascending
  `obu_xlayer_id` ordering.
