# validator spec delta

## ADDED Requirements

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
