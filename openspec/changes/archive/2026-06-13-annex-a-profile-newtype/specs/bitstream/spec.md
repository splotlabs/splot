# bitstream delta: annex-a-profile-newtype

Advances `AV2-A-PROFILES` (`types` stage) by modeling the profile identifier as a strong
enum over the Annex A.2 Table A.1 value space.

## ADDED Requirements

### Requirement: profile identifier strong type

The `ProfileIdc` type SHALL model the `seq_profile_idc` / `multistream_profile_idc` value
space as an enum over Annex A.2 Table A.1
(docs/spec/av2/1.0.0/annex-a-profiles-levels-and-tiers.md, mirror lines 59-90): the five
defined Main profiles (`seq_profile_idc` 0-4), the reserved range (`5..=30`, preserving the
raw value), and the Configurable profile (31). `from_bits` maps every 5-bit value and `get`
returns the raw value (round-trip), and the type's ordering matches the raw value order. The
MSDO `multistream_profile_idc`, OPS `ops_seq_profile_idc`, and LCR `lcr_seq_profile_idc`
fields use this type.

#### Scenario: round-trip and classification

- **WHEN** a 5-bit profile value is parsed into `ProfileIdc`
- **THEN** `get()` returns the original value, and the variant identifies a defined Main
  profile, the reserved range, or the Configurable profile per Table A.1

#### Scenario: behavior preserved

- **WHEN** the validator consumes a profile field
- **THEN** the conformance verdicts are identical to the prior raw-`u8` representation (the
  refactor changes types only, not behavior)

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
