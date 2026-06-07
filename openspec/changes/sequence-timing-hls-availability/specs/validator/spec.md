# validator spec delta

## ADDED Requirements

### Requirement: Cross-embedded-layer timing consistency

`splot-validate` SHALL compare timing information (`timing_info()`, reached through
the content-interpretation OBU) across embedded layers of the same coded video
sequence, and flag inconsistencies (AV2 v1.0.0 § 6.4.12). The comparison SHALL be
made only between two timing values that are both present and both decidably within
the same modeled coded-video-sequence scope.

#### Scenario: matching timing across embedded layers is accepted

- **GIVEN** two content-interpretation OBUs for the same extended layer but
  different embedded layers, both carrying timing information
- **WHEN** their `num_units_in_display_tick`, `time_scale`,
  `equal_picture_interval`, and `num_ticks_per_picture_minus_1` values are equal
- **THEN** validation SHALL NOT emit any `sequence-header/timing-*-mismatch`.

#### Scenario: mismatched display tick across embedded layers

- **GIVEN** two embedded layers in one coded video sequence that both carry timing
  information
- **WHEN** their `num_units_in_display_tick` values differ
- **THEN** validation SHALL emit `sequence-header/timing-display-tick-mismatch`.

#### Scenario: mismatched time scale across embedded layers

- **GIVEN** two embedded layers in one coded video sequence that both carry timing
  information
- **WHEN** their `time_scale` values differ
- **THEN** validation SHALL emit `sequence-header/timing-time-scale-mismatch`.

#### Scenario: mismatched equal-picture-interval across embedded layers

- **GIVEN** two embedded layers in one coded video sequence that both carry timing
  information
- **WHEN** their `equal_picture_interval` values differ
- **THEN** validation SHALL emit
  `sequence-header/timing-equal-picture-interval-mismatch`.

#### Scenario: mismatched ticks-per-picture across embedded layers

- **GIVEN** two embedded layers in one coded video sequence that both carry timing
  information with `equal_picture_interval` equal to 1
- **WHEN** their `num_ticks_per_picture_minus_1` values differ
- **THEN** validation SHALL emit `sequence-header/timing-num-ticks-mismatch`.

#### Scenario: timing not yet comparable

- **GIVEN** at most one embedded layer carries present timing information in the
  modeled coded-video-sequence scope
- **WHEN** validation runs
- **THEN** the validator SHALL NOT fabricate a timing-mismatch diagnostic.

### Requirement: Content-interpretation range conformance

`splot-validate` SHALL enforce the locally-decidable § 6.14 range constraints of the
content-interpretation OBU.

#### Scenario: chroma sample position out of range

- **GIVEN** a content-interpretation OBU with `ci_chroma_sample_position_top` or
  `ci_chroma_sample_position_bottom` greater than 5
- **WHEN** validation runs
- **THEN** validation SHALL emit
  `content-interpretation/chroma-sample-position-out-of-range`.

#### Scenario: aspect ratio idc out of range

- **GIVEN** a content-interpretation OBU with `ci_aspect_ratio_idc` not equal to 255
  and greater than 16
- **WHEN** validation runs
- **THEN** validation SHALL emit
  `content-interpretation/aspect-ratio-idc-out-of-range`.

### Requirement: Repeated content-interpretation identity

`splot-validate` SHALL flag a content-interpretation OBU that is repeated for the
same embedded layer within the modeled coded-video-sequence scope carrying different
*information*, where the decoder-ignored `ci_reserved_2bit` is normalized out of the
comparison (AV2 v1.0.0 § 6.14: a repeated CI OBU must "contain the same
information").

#### Scenario: repeated non-identical content interpretation

- **GIVEN** two content-interpretation OBUs for the same `(obu_xlayer_id,
  obu_mlayer_id)` within one coded video sequence
- **WHEN** their parsed § 5.15 information differs (other than `ci_reserved_2bit`)
- **THEN** validation SHALL emit `content-interpretation/repeated-ci-not-identical`.

#### Scenario: repeat differing only in reserved bits

- **GIVEN** two content-interpretation OBUs for the same `(obu_xlayer_id,
  obu_mlayer_id)` whose parsed § 5.15 fields are identical except `ci_reserved_2bit`
- **WHEN** validation runs
- **THEN** validation SHALL NOT emit `content-interpretation/repeated-ci-not-identical`.

### Requirement: Content-interpretation reserved bits

`splot-validate` SHALL surface a non-zero `ci_reserved_2bit` (AV2 v1.0.0 § 6.14).

#### Scenario: non-zero reserved bits

- **GIVEN** a content-interpretation OBU whose `ci_reserved_2bit` is not 0
- **WHEN** validation runs
- **THEN** validation SHALL emit `content-interpretation/reserved-bits-nonzero` as a
  warning (the value is ignored by a decoder, so it is not a hard error).

### Requirement: HLS availability store

`splot-validate` SHALL model in-band availability of sequence-header HLS objects
before they are referenced, with optional caller-provided external HLS supplied
through `ValidationOptions` (AV2 v1.0.0 § 7.3.8). The default `ValidationOptions`
SHALL NOT assume any external HLS is available.

#### Scenario: multi-frame header references an available sequence header

- **GIVEN** a sequence-header OBU with `seq_header_id` equal to id earlier in the
  bitstream
- **AND** a later multi-frame header OBU with `mfh_seq_header_id` equal to id
- **WHEN** validation reaches the reference
- **THEN** validation SHALL NOT emit `mfh/sequence-header-unavailable`.

#### Scenario: multi-frame header references an unavailable sequence header

- **GIVEN** a multi-frame header OBU with `mfh_seq_header_id` equal to id
- **AND** no in-band or caller-provided sequence header with that id is available
- **WHEN** validation reaches the reference
- **THEN** validation SHALL emit `mfh/sequence-header-unavailable`.

#### Scenario: external HLS provides the referenced sequence header

- **GIVEN** a multi-frame header OBU with `mfh_seq_header_id` equal to id
- **AND** no in-band sequence header with that id, but caller-provided external HLS
  declares id available
- **WHEN** validation runs with `ExternalHlsMode::Provided`
- **THEN** validation SHALL NOT emit `mfh/sequence-header-unavailable`.

#### Scenario: external HLS disabled advisory

- **GIVEN** a multi-frame header reference that cannot be satisfied in-band
- **WHEN** validation runs with the default `ExternalHlsMode::Disabled`
- **THEN** validation SHALL emit `mfh/sequence-header-unavailable`
- **AND** SHALL additionally emit the advisory `hls/external-hls-disabled`.

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
