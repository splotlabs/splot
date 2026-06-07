# bitstream spec delta

## ADDED Requirements

### Requirement: Rice-Golomb descriptor

`splot-core` SHALL provide a panic-free `rg(n)` descriptor reader (AV2 v1.0.0
§ 4.11.10) for use by the content-interpretation OBU.

#### Scenario: well-formed rg(n) value

- **GIVEN** a bit reader positioned at an `rg(n)` code with a unary prefix that
  terminates within 32 bits
- **WHEN** the descriptor is read
- **THEN** it SHALL return `(q << n) + remainder`, where `q` is the number of
  leading one bits and `remainder` is the `n`-bit suffix.

#### Scenario: non-terminating rg(n) prefix

- **GIVEN** a bit reader whose next 32 bits are all one
- **WHEN** the descriptor is read
- **THEN** it SHALL return a typed error (the spec requires the descriptor never
  return a value less than 0) and SHALL NOT panic.

### Requirement: Content interpretation OBU parser

`splot-core` SHALL parse `content_interpretation_obu()` (AV2 v1.0.0 § 5.15) into
typed fields, reaching `timing_info()` when `ci_timing_info_present_flag` is set,
and SHALL be dispatched from `open_bitstream_unit(sz)`.

#### Scenario: content interpretation with timing is parsed

- **GIVEN** an Annex B bitstream containing an `OBU_CONTENT_INTERPRETATION`
- **AND** `ci_timing_info_present_flag` equal to 1 with valid timing
- **WHEN** the OBU is dispatched by `open_bitstream_unit(sz)`
- **THEN** the syntax SHALL be parsed into typed Rust fields including the present
  `timing_info()`
- **AND** the parser SHALL NOT read past the declared OBU payload.

#### Scenario: content interpretation optional branches

- **GIVEN** an `OBU_CONTENT_INTERPRETATION` with any combination of
  `ci_color_description_present_flag`, `ci_chroma_sample_position_present_flag`, and
  `ci_aspect_ratio_info_present_flag` set
- **WHEN** the OBU is parsed
- **THEN** each present branch (including the `rg(2)` color-description id, the
  chroma-sample-position UVLC fields, and the extended `ci_sar_width`/
  `ci_sar_height` path when `ci_aspect_ratio_idc == 255`) SHALL be read into typed
  fields without skipping unknown bits.

#### Scenario: content interpretation truncated mid-field

- **GIVEN** an `OBU_CONTENT_INTERPRETATION` whose payload ends inside the fixed
  header or inside `timing_info()`
- **WHEN** the OBU is parsed
- **THEN** the parser SHALL return a structured error
- **AND** the validator SHALL convert it to a diagnostic rather than panicking.

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
