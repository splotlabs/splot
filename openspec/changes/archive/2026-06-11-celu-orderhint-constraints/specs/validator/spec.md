# validator delta: celu-orderhint-constraints

Advances `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT` and the § 7.3.7 DOH checks.

## ADDED Requirements

### Requirement: coded-extended-layer-unit structure

The validator SHALL enforce the § 7.3.6 in-unit OBU order (layer
configuration records, operating point sets, atlas segments, sequence
headers, then per-embedded-layer frame units in ascending `obu_mlayer_id`,
with PADDING position-free) and the § 7.3.6 constraint family: at least one
coded output frame unit, non-output-implies-output per embedded layer, one
OrderHint across all output units, the CLK/OLK first-frame-unit and
lowest-layer rules, no CLK+OLK mix, all-leading-or-none, and
content-interpretation only in each layer's first frame unit. Units whose
classification is Unknown SHALL NOT fire.

#### Scenario: sequence header after a frame unit

- **WHEN** a CELU carries a sequence header after its first coded frame unit
  began
- **THEN** a `celu/` ordering error citing § 7.3.6 is emitted

#### Scenario: output units disagree on OrderHint

- **WHEN** two coded output frame units in one CELU carry different parsed
  `order_hint` values
- **THEN** an error citing § 7.3.6 is emitted

#### Scenario: CLK and OLK mixed

- **WHEN** one CELU contains both a CLK and an OLK OBU
- **THEN** an error citing § 7.3.6 is emitted

### Requirement: DOH-gated OrderHint agreement

The validator SHALL enforce the § 7.3.7 DOH constraints when the recorded
`multistream_doh_constraint_flag` or `lcr_doh_constraint_flag` equals 1: one
OrderHintBits for all frame units in the temporal unit and one OrderHint
across the coded output frame units of the temporal unit's CELUs; with the
flag 0 the checks SHALL stay silent.

#### Scenario: cross-CELU OrderHint mismatch under the DOH flag

- **WHEN** the DOH flag is 1 and two CELUs' output frame units in one
  temporal unit carry different OrderHint values
- **THEN** an error citing § 7.3.7 is emitted

#### Scenario: flag off stays silent

- **WHEN** no DOH constraint flag is set
- **THEN** no DOH OrderHint diagnostic is emitted

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
