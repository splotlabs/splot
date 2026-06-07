# validator Specification

## Purpose

Parser-driven conformance diagnostics in `splot-validate`. Diagnostics are the
product: every finding is structured data (stable `rule_id`, `severity`, optional
`spec_section`, optional byte/bit offset, human-readable `message`). A malformed
bitstream is a report, never a process failure.

Tracked by Feature IDs: `AV2-5.2.2-OBU-HEADER` (header constraints),
`AV2-5.3-RESERVED-OBU`, `AV2-7.3-OBU-ORDERING`.
## Requirements
### Requirement: structured diagnostics

Every check SHALL emit `Diagnostic`s with a stable `rule_id`, a `severity`, the AV2
`spec_section` where applicable, and a byte offset where known.

#### Scenario: global xlayer constraint

- **WHEN** an `OBU_TEMPORAL_DELIMITER` has `obu_xlayer_id != GLOBAL_XLAYER_ID`
- **THEN** an error diagnostic `obu-header/global-xlayer-required` (§ 6.2.2) is produced

### Requirement: reserved OBU handling

A reserved OBU SHALL be reported informationally; a reserved OBU whose payload is
entirely zero SHALL be an error (AV2 v1.0.0 § 5.3 / § 6.2.3 require a non-zero
trailing bit).

#### Scenario: all-zero reserved payload

- **WHEN** a reserved OBU carries an entirely-zero payload
- **THEN** an error diagnostic `obu-reserved/all-zero-payload` is produced

### Requirement: diagnostic rule-id namespace

Diagnostic rule ids SHALL use a documented kebab/slash prefix (`obu-header/`,
`obu-reserved/`, `bitstream/`). Narrower diagnostics derived from a modeled feature
MAY use the Feature ID as a base with a `.SUFFIX`.

#### Scenario: undocumented prefix is rejected

- **WHEN** a diagnostic rule id uses a prefix that is not documented
- **THEN** `cargo xtask check-feature-status` fails

### Requirement: Sequence header semantic diagnostics

`splot-validate` SHALL emit stable diagnostics for locally decidable §6.4 sequence-header semantic violations covered by implemented parsers.

#### Scenario: zero timing values

- **GIVEN** a sequence header with timing information present
- **WHEN** `num_units_in_display_tick == 0` or `time_scale == 0`
- **THEN** validation SHALL emit a `sequence-header/` diagnostic with severity `error` and the relevant AV2 section.

### Requirement: Activated sequence layer limits

`splot-validate` SHALL use available activated sequence headers to validate OBU layer identifiers.

#### Scenario: temporal layer exceeds active sequence maximum

- **GIVEN** an active sequence header for an extended layer
- **AND** a subsequent non-global OBU associated with that layer
- **WHEN** the OBU has `obu_tlayer_id > max_tlayer_id`
- **THEN** validation SHALL emit `sequence-state/tlayer-exceeds-max`.

#### Scenario: embedded layer exceeds active sequence maximum

- **GIVEN** an active sequence header for an extended layer
- **AND** a subsequent non-global OBU associated with that layer
- **WHEN** the OBU has `obu_mlayer_id > max_mlayer_id`
- **THEN** validation SHALL emit `sequence-state/mlayer-exceeds-max`.

### Requirement: HLS availability state

`splot-validate` SHALL model in-band HLS availability before an OBU references sequence/HLS state.

#### Scenario: unavailable sequence header

- **GIVEN** an OBU or HLS object references a sequence-header id
- **AND** no matching in-band or caller-provided external sequence header is available
- **WHEN** validation reaches the reference
- **THEN** validation SHALL emit `hls/unavailable-sequence-header`.

### Requirement: Temporal-unit ordering

`splot-validate` SHALL continue to enforce the implemented subset of AV2 temporal-unit order.

#### Scenario: duplicate temporal delimiter

- **GIVEN** a temporal unit with a global temporal delimiter already seen
- **WHEN** another global temporal delimiter appears before the next temporal unit begins
- **THEN** validation SHALL emit `obu-order/duplicate-temporal-delimiter`.

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

### Requirement: Layer configuration record and atlas availability

`splot-validate` SHALL track in-band layer-configuration-record and local
atlas-segment availability and emit diagnostics when a reference cannot be resolved
(AV2 § 7.3.8.3 / § 7.3.8.4), gating the hard errors on external HLS being disabled. The
global atlas (§ 7.3.8.4 "can be available") SHALL NOT be flagged when missing.

#### Scenario: local LCR references an unavailable global LCR

- **GIVEN** a local LCR whose `lcr_global_id` is non-zero and no preceding global LCR
  has that `lcr_global_config_record_id`
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit a `lcr/global-lcr-unavailable` error.

#### Scenario: local LCR references an unavailable local atlas

- **GIVEN** a local LCR whose `lcr_local_atlas_id` has no preceding local atlas segment
  OBU in the same extended layer
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit an `atlas/local-atlas-unavailable` error.

### Requirement: Sequence header `seq_lcr_id` resolution

`splot-validate` SHALL resolve a sequence header's `seq_lcr_id` (when non-zero) to an
available local LCR (same xlayer) or, failing that, an available global LCR whose
`lcr_xlayer_map` includes the sequence header's xlayer (AV2 § 6.4.1 / § 7.3.8.6).

#### Scenario: seq_lcr_id resolves to no LCR

- **GIVEN** a sequence header with `seq_lcr_id != 0` and no matching LCR
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit a `hls/unavailable-layer-configuration-record` error.

#### Scenario: global LCR omits the header's xlayer

- **GIVEN** a sequence header whose `seq_lcr_id` resolves to a global LCR whose
  `lcr_xlayer_map` does not include the header's `obu_xlayer_id`
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit a `lcr/global-xlayer-map-missing-xlayer` error.

