# validator spec delta

## ADDED Requirements

### Requirement: Cross-embedded-layer timing consistency

`splot-validate` SHALL compare timing information across embedded layers of the same
coded video sequence once `timing_info()` is reachable, and flag inconsistencies
(AV2 v1.0.0 § 6.4.12).

#### Scenario: mismatched time scale across embedded layers

- **GIVEN** two embedded layers in one coded video sequence that both carry timing
  information
- **WHEN** their `time_scale` values differ
- **THEN** validation SHALL emit `sequence-header/timing-time-scale-mismatch`.

#### Scenario: timing not yet reachable

- **GIVEN** a bitstream whose timing information is not parseable because the
  content-interpretation OBU is not yet modeled
- **WHEN** validation runs
- **THEN** the validator SHALL NOT fabricate a timing diagnostic and SHALL leave the
  check bounded.

### Requirement: HLS availability store

`splot-validate` SHALL model availability of HLS objects (sequence headers, MSDO,
multi-frame headers) before they are referenced, with optional caller-provided
external HLS (AV2 v1.0.0 § 7.3.8).

#### Scenario: multi-frame header references an unavailable sequence header

- **GIVEN** a multi-frame header OBU with `mfh_seq_header_id` equal to id
- **AND** no in-band or caller-provided sequence header with that id is available
- **WHEN** validation reaches the reference
- **THEN** validation SHALL emit `mfh/sequence-header-unavailable`.

#### Scenario: external HLS required but disabled

- **GIVEN** a validation run with external HLS disabled
- **WHEN** an OBU references an HLS object available only externally
- **THEN** validation SHALL emit `hls/external-hls-disabled`.

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
