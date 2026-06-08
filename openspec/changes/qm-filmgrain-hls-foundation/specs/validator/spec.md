# validator spec delta

## ADDED Requirements

### Requirement: Quantizer Matrix duplicate-reset validation

`splot-validate` SHALL report an error when a quantizer matrix OBU with
`qm_bit_map == 0` is not the first quantizer matrix OBU between coded frames
(AV2 v1.0.0 § 6.12).

#### Scenario: duplicate reset

- **GIVEN** two quantizer matrix OBUs between coded-frame boundaries, both with
  `qm_bit_map == 0`
- **WHEN** the stream is validated
- **THEN** the validator SHALL emit `qm/duplicate-reset-between-frames`.

#### Scenario: single reset is conformant

- **GIVEN** a single quantizer matrix OBU with `qm_bit_map == 0` between coded frames
- **WHEN** the stream is validated
- **THEN** the validator SHALL NOT emit `qm/duplicate-reset-between-frames`.

### Requirement: Quantizer Matrix duplicate-level validation

`splot-validate` SHALL report an error when the same quantizer matrix level is specified
twice between coded frames (AV2 v1.0.0 § 6.12).

#### Scenario: duplicate level

- **GIVEN** two quantizer matrix OBUs between coded-frame boundaries that both specify
  level `L`
- **WHEN** the stream is validated
- **THEN** the validator SHALL emit `qm/duplicate-level-between-frames`.

#### Scenario: same level across a coded frame is allowed

- **GIVEN** two quantizer matrix OBUs that specify level `L` but are separated by a
  frame-bearing OBU
- **WHEN** the stream is validated
- **THEN** the validator SHALL NOT emit `qm/duplicate-level-between-frames`.

### Requirement: Quantizer Matrix HLS availability state

`splot-validate` SHALL record per-level quantizer-matrix availability for future
frame-reference validation.

#### Scenario: user-defined level parsed

- **WHEN** a quantizer matrix OBU specifies level `L`
- **THEN** the validator SHALL record level `L` with its defining layer identity, plane
  count, and data-present status.

### Requirement: Film grain update-flags validation

`splot-validate` SHALL report an error when `fgm_update_flags == 0` (AV2 v1.0.0 § 6.13).

#### Scenario: empty film-grain update

- **WHEN** a film grain OBU has `fgm_update_flags == 0`
- **THEN** the validator SHALL emit `film-grain/update-flags-zero`.

### Requirement: Film grain chroma-idc validation

`splot-validate` SHALL report an error when `fgm_chroma_idc > 3` (AV2 v1.0.0 § 6.13).

#### Scenario: out-of-range chroma idc

- **WHEN** a film grain OBU has `fgm_chroma_idc` greater than `3`
- **THEN** the validator SHALL emit `film-grain/chroma-idc-out-of-range`.

### Requirement: Film grain duplicate-slot validation

`splot-validate` SHALL report an error when the same film-grain slot is updated more
than once in the same coded frame unit, subject to the validator's coded-frame-unit
boundary model (AV2 v1.0.0 § 6.13).

#### Scenario: duplicate slot in one coded frame unit

- **GIVEN** two film grain OBUs in the same coded frame unit that both update slot `i`
- **WHEN** the stream is validated
- **THEN** the validator SHALL emit `film-grain/duplicate-slot-in-coded-frame-unit`.

### Requirement: Film grain HLS availability state

`splot-validate` SHALL record per-slot film-grain availability for future
frame-reference validation.

#### Scenario: slot updated

- **WHEN** a film grain OBU updates slot `i`
- **THEN** the validator SHALL record slot `i` with its defining layer identity and
  chroma format.

### Requirement: Deferred frame-reference validation

`splot-validate` SHALL NOT claim frame-reference validation for quantizer matrices or
film grain (`using_qmatrix` / `qm_*`, `apply_grain` / `fgm_id`) until the relevant
frame-header fields are parsed and proven.

#### Scenario: no frame-reference diagnostics this phase

- **WHEN** a stream contains quantizer-matrix or film-grain OBUs without a parsed frame
  header
- **THEN** the validator SHALL NOT emit any `qm/unavailable-*` or `film-grain/unavailable-*`
  diagnostics.
