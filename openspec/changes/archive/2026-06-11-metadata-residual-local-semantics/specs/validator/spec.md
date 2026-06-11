# validator delta: metadata-residual-local-semantics

Advances `AV2-5.17.7-METADATA-TIMECODE` (and the frame-hash reserved-field
fact if one is missing).

## ADDED Requirements

### Requirement: timecode inference requires a previous value

The validator SHALL emit an error when a § 6.16.7 timecode omits
`seconds_value`, `minutes_value`, or `hours_value` and no previous set of
clock timestamp syntax elements in decoding order carried that value, per the
mirror's "it is required that such a previous … shall have been present"
sentences. The decoding-order chain is keyed per the carrying OBU's concrete
`(obu_xlayer_id, obu_mlayer_id)`: METADATA_TYPE_TIMECODE is layer-specific
(§ 6.16.3 Table 6.17), so a timecode on one embedded layer is not the
"previous set" of one on a different embedded layer and must not seed its
inference; a `LAYER_UNSPECIFIED` timecode chains per its own carrying scope.

#### Scenario: inferred seconds without any previous timecode

- **WHEN** the first timecode in scope omits `seconds_value`
  (`full_timestamp_flag = 0`, `seconds_flag = 0`)
- **THEN** `metadata/timecode-inferred-without-previous` (error, § 6.16.7)
  is emitted naming `seconds_value`

#### Scenario: inference after a present value passes

- **WHEN** a timecode omits `seconds_value` after a previous set carried it
- **THEN** no inference diagnostic is emitted

#### Scenario: inference is keyed per targeted embedded layer

- **WHEN** a full-timestamp `LAYER_CURRENT` timecode on `(obu_xlayer_id 0,
  obu_mlayer_id 0)` is followed by a `LAYER_CURRENT` timecode on `(obu_xlayer_id
  0, obu_mlayer_id 1)` that omits `seconds_value`
- **THEN** `metadata/timecode-inferred-without-previous` (error, § 6.16.7) is
  emitted: the `(0, 0)` timecode is not the previous set for `(0, 1)`

### Requirement: timecode n_frames bound

The validator SHALL emit an error when a § 6.16.7 timecode's `n_frames` is not
less than `maxPicPerSecond` (`ceil(time_scale / TicksPerPicture)`) and an
in-scope content interpretation establishes `ci_timing_info_present_flag == 1`
at or after the layer's § 7.3.8.11 random-access-point epoch, per the mirror's
"When ci_timing_info_present_flag is equal to 1, n_frames shall be less than
maxPicPerSecond". The bound is paired against the in-scope CI timing in both
arrival orders (a content interpretation arriving after the timecode
re-evaluates), and the diagnostic anchors at the offending timecode metadata
OBU. The § 6.16.3 layer targeting scopes the pairing: a derivable `LAYER_VALUES`
timecode naming only some embedded layers does not pair with an untargeted
layer's CI, while a timecode whose targeting is not bitstream-derivable
(`LAYER_UNSPECIFIED`) compares nothing for the bound — the spec leaves its layer
association unspecified, so no CI's rate binds it. § 7.3.6 coded-video-sequence
boundaries are per extended layer: a CLK for one extended layer does not prune a
global timecode observation aimed at another extended layer. The CI-re-send
dedup is epoch-aware: an identical CI repeated in a later temporal unit with no
random access point in between does not re-report the already-paired
observation, while a CI re-sent in a random-access temporal unit re-pairs the
new coded video sequence's observations at the CLK.

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

#### Scenario: unspecified targeting compares nothing

- **WHEN** a `LAYER_UNSPECIFIED` timecode whose `n_frames` would exceed an
  extended-layer-0 CI's low-rate `maxPicPerSecond` is observed
- **THEN** no `metadata/timecode-n-frames-exceeds-rate` diagnostic is emitted
  (the spec does not say which layers the timecode applies to, so no CI's rate
  binds it — a zero-false-positive rule)

#### Scenario: a global observation survives an unrelated layer's CLK

- **WHEN** a global `LAYER_VALUES` timecode targeting extended layer 1 is
  observed, a CLK for extended layer 0 only follows, and a low-rate CI for
  extended layer 1 that the `n_frames` exceeds then arrives
- **THEN** `metadata/timecode-n-frames-exceeds-rate` (error, § 6.16.7) is
  emitted: the extended-layer-0 CLK does not prune the layer-1 observation

#### Scenario: an identical CI repeat with no random access point reports once

- **WHEN** a CI establishes a low-rate timing and a violating timecode is
  reported, then the identical CI is re-sent in a later temporal unit with no
  CLK or OLK in between
- **THEN** `metadata/timecode-n-frames-exceeds-rate` is emitted exactly once
  (the epoch-aware dedup does not replay the recheck for the already-paired
  observation)

#### Scenario: a CI re-sent across a random access point still pairs

- **WHEN** a pre-RAP CI establishes a low-rate timing, a later random-access
  temporal unit holds a timecode that violates the bound followed by the same CI
  re-sent with identical timing and then a CLK, and the deferred pre-RAP pairing
  is dropped by the § 7.3.8.11 reinitialization
- **THEN** `metadata/timecode-n-frames-exceeds-rate` (error, § 6.16.7) is still
  emitted, anchored at the timecode OBU (the new coded video sequence's timecode
  is re-paired against the re-sent CI at the CLK)

### Requirement: timecode counting_type reserved value

The validator SHALL warn when a § 6.16.7 `counting_type` is in the reserved
range 7..31. The counting_type table marks those values "reserved" with no
"shall" forbidding them (§ 6.16.7 only recommends counting_type "should be the
same for all pictures"), so a reserved value is a decoder-ignored producer
anomaly (warning), matching the established reserved-value pattern.

#### Scenario: reserved counting_type is warned

- **WHEN** a timecode carries `counting_type == 7`
- **THEN** `metadata/timecode-counting-type-reserved` (warning, § 6.16.7) is
  emitted and the report stays conformant (no error)

#### Scenario: a defined counting_type is silent

- **WHEN** a timecode carries `counting_type == 6` (the highest defined value)
- **THEN** no `metadata/timecode-counting-type-reserved` diagnostic is emitted

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
