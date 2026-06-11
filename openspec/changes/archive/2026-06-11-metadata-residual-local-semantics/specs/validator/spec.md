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

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
