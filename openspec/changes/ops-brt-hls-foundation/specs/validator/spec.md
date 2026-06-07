# validator spec delta

## ADDED Requirements

### Requirement: Active operating point set state

`splot-validate` SHALL maintain active in-band operating point set records keyed by
`(obu_xlayer_id, ops_id)` with the non-monotonic reset/update semantics of AV2 v1.0.0
§ 6.10.1, distinct from the monotonic HLS availability store.

#### Scenario: reset clears active OPS

- **GIVEN** an OPS that defines `(xlayer, ops_id)`, followed by an OPS OBU with
  `ops_reset_flag == 1` and `ops_cnt == 0` for that layer
- **WHEN** the validator runs
- **THEN** the previously defined OPS SHALL no longer be available
- **AND** a later buffer-removal-timing reference to it SHALL be unavailable.

#### Scenario: update changes the active count

- **GIVEN** an OPS defined with `ops_cnt == 2` that is then redefined with
  `ops_cnt == 3`
- **WHEN** a buffer-removal-timing OBU references it with `br_ops_cnt == 2`
- **THEN** the validator SHALL compare against the updated `ops_cnt == 3`.

### Requirement: Locally-decidable OPS semantics

`splot-validate` SHALL emit `ops/*` diagnostics for the locally-decidable § 6.10
conformance violations.

#### Scenario: local reserved bits

- **GIVEN** a local OPS with a non-zero `ops_reserved_2bits`
- **WHEN** the validator runs
- **THEN** it SHALL emit an `ops/local-reserved-bits-nonzero` error.

#### Scenario: reserved mlayer-info idc

- **GIVEN** a global OPS with `ops_mlayer_info_idc == 3`
- **WHEN** the validator runs
- **THEN** it SHALL emit an `ops/mlayer-info-idc-reserved` error.

#### Scenario: payload size mismatch

- **GIVEN** an operating point payload whose computed `opsBytes` differs from its
  declared `ops_data_size`
- **WHEN** the validator runs
- **THEN** it SHALL emit an `ops/payload-size-mismatch` error.

#### Scenario: inherited op-index out of range

- **GIVEN** an inherited operating-point reference whose `ops_embedded_op_index` is out
  of range for the referenced operating point set
- **WHEN** the validator runs
- **THEN** it SHALL emit an `ops/inherited-op-index-out-of-range` error.

### Requirement: Buffer removal timing references

`splot-validate` SHALL validate OPS-dependent buffer-removal-timing references against
active OPS state, gating hard errors on external HLS being disabled.

#### Scenario: unavailable operating point set

- **GIVEN** an OPS-dependent BRT whose `br_ops_id` resolves to no active OPS
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit a `brt/unavailable-operating-point-set` error.

#### Scenario: count mismatch

- **GIVEN** an OPS-dependent BRT whose `br_ops_cnt` differs from the active OPS
  `ops_cnt`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `brt/ops-count-mismatch` error.

#### Scenario: external HLS suppresses the hard missing-OPS error

- **GIVEN** an OPS-dependent BRT whose `br_ops_id` resolves to no in-band OPS
- **WHEN** the validator runs with external HLS provided
- **THEN** it SHALL NOT emit a hard `brt/unavailable-operating-point-set` error.

### Requirement: Buffer removal timing ordering classification

`splot-validate` SHALL classify `OBU_BUFFER_REMOVAL_TIMING` for temporal-unit ordering
per AV2 § 7.3.3 / § 7.3.4 / § 7.3.7: a local BRT is a coded-extended-layer OBU, and a
global BRT is not a global temporal-unit prefix OBU.

#### Scenario: local BRT starts the coded-layer phase

- **GIVEN** a local BRT followed by a global OPS within a temporal unit
- **WHEN** the validator runs
- **THEN** it SHALL flag the global OPS with `obu-order/global-hls-after-coded-layer`,
  because the local BRT started the coded-layer phase.

#### Scenario: global BRT is not flagged for ordering

- **GIVEN** a global BRT before or after a coded extended layer unit
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit an `obu-order/global-hls-after-coded-layer` error for the
  BRT.
