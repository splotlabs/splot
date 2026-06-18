## ADDED Requirements

### Requirement: Ordinary non-FSC coefficient pass composition
The decoder SHALL provide a crate-private ordinary non-FSC coefficient helper
that composes the existing AV2 § 5.20.7.27 and § 5.20.7.28 boundaries from
nonzero block start through signed `Quant[]` writes.

#### Scenario: Existing coefficient boundaries compose
- **GIVEN** a decoded nonzero EOB block start, a caller-resolved scan table,
  caller-resolved base-symbol inputs, caller-resolved sign inputs,
  caller-resolved plane and transform-class facts, and block-level quant-pass
  hidden/sumAbs1/TCQ/lossless facts
- **WHEN** the composed helper runs
- **THEN** it walks the checked scan entries
- **AND** it reads base/base-range coefficient symbols through the existing
  base-symbol helper
- **AND** it writes decoded levels through the existing local `Level[]` helper
- **AND** it resets `hrLevelAvg` to 0 at coefficient-block entry before
  `read_quant`
- **AND** for each checked coefficient it reads that coefficient's sign, derives
  its `maxLevel`, reads `read_quant`, and writes its signed `Quant[]` value
  before moving to the next coefficient
- **AND** it returns the intermediate scan, base-read, sign-read, and quant-pass
  summaries

#### Scenario: Preflight failures preserve later state
- **GIVEN** malformed caller facts such as a scan table shorter than decoded EOB,
  mismatched base inputs, invalid sign inputs, or inconsistent quant-pass facts
- **WHEN** the composed helper is called
- **THEN** it returns a typed crate-private error at the failed boundary
- **AND** later coefficient phases are not run after the first failure

#### Scenario: Runtime decode remains unchanged
- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the composed ordinary pass helper yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the helper
