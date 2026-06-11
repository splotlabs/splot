# validator delta: metadata-residual-local-semantics

Advances `AV2-5.17.7-METADATA-TIMECODE` (and the frame-hash reserved-field
fact if one is missing).

## ADDED Requirements

### Requirement: timecode inference requires a previous value

The validator SHALL emit an error when a § 6.16.7 timecode omits
`seconds_value`, `minutes_value`, or `hours_value` and no previous set of
clock timestamp syntax elements in decoding order carried that value, per the
mirror's "it is required that such a previous … shall have been present"
sentences.

#### Scenario: inferred seconds without any previous timecode

- **WHEN** the first timecode in scope omits `seconds_value`
  (`full_timestamp_flag = 0`, `seconds_flag = 0`)
- **THEN** `metadata/timecode-inferred-without-previous` (error, § 6.16.7)
  is emitted naming `seconds_value`

#### Scenario: inference after a present value passes

- **WHEN** a timecode omits `seconds_value` after a previous set carried it
- **THEN** no inference diagnostic is emitted

### Requirement: timecode n_frames bound

The validator SHALL emit an error when a § 6.16.7 timecode's `n_frames` is not
less than `maxPicPerSecond` (`ceil(time_scale / TicksPerPicture)`) and an
in-scope content interpretation establishes `ci_timing_info_present_flag == 1`
at or after the layer's § 7.3.8.11 random-access-point epoch, per the mirror's
"When ci_timing_info_present_flag is equal to 1, n_frames shall be less than
maxPicPerSecond". The bound is paired against the in-scope CI timing in both
arrival orders (a content interpretation arriving after the timecode
re-evaluates), and the diagnostic anchors at the offending timecode metadata
OBU. The § 6.16.3 layer targeting scopes the pairing: a `LAYER_VALUES` timecode
naming only some embedded layers does not pair with an untargeted layer's CI.

#### Scenario: n_frames at the rate ceiling is flagged

- **WHEN** a timecode carries `n_frames == maxPicPerSecond` and an in-scope CI
  establishes the timing
- **THEN** `metadata/timecode-n-frames-exceeds-rate` (error, § 6.16.7) is
  emitted, anchored at the timecode OBU

#### Scenario: n_frames just below the ceiling passes

- **WHEN** a timecode carries `n_frames == maxPicPerSecond - 1`
- **THEN** no `metadata/timecode-n-frames-exceeds-rate` diagnostic is emitted

#### Scenario: targeting excludes an untargeted layer's CI

- **WHEN** a `LAYER_VALUES` timecode targets embedded layer 1 only, embedded
  layer 0 carries a low-rate CI the `n_frames` would exceed, and embedded layer
  1 carries a CI under which the `n_frames` is legal
- **THEN** no `metadata/timecode-n-frames-exceeds-rate` diagnostic is emitted

#### Scenario: a CI re-sent across a random access point still pairs

- **WHEN** a pre-RAP CI establishes a low-rate timing, a later random-access
  temporal unit holds a timecode that violates the bound followed by the same CI
  re-sent with identical timing and then a CLK, and the deferred pre-RAP pairing
  is dropped by the § 7.3.8.11 reinitialization
- **THEN** `metadata/timecode-n-frames-exceeds-rate` (error, § 6.16.7) is still
  emitted, anchored at the timecode OBU (the temporal-unit-scoped dedup does not
  suppress the re-pair of the new epoch's timecode against the re-sent CI)

### Requirement: decoded frame hash reserved field

The validator SHALL warn when a § 6.16.13 `metadata_decoded_frame_hash()` carries
a non-zero `reserved` bit, per the mirror's "reserved shall be set to 0 and
ignored by decoders". The bit is decoder-ignored, so the finding is a producer
anomaly (warning), matching the established decoder-ignored reserved-field
pattern; the `plane_hash` / `frame_hash` verification against decoded output
stays decoder-blocked.

#### Scenario: non-zero reserved bit is warned

- **GIVEN** a `metadata_decoded_frame_hash()` OBU whose `reserved` bit is 1
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/decoded-frame-hash-reserved-nonzero`
  warning (§ 6.16.13)

#### Scenario: zero reserved bit is silent

- **GIVEN** a `metadata_decoded_frame_hash()` OBU whose `reserved` bit is 0
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit a `metadata/decoded-frame-hash-reserved-nonzero`
  warning

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
