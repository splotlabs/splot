## ADDED Requirements

### Requirement: Ordinary non-FSC maxLevel derivation
The decoder SHALL provide a crate-private ordinary non-FSC coefficient helper
that derives AV2 § 5.20.7.27 `maxLevel` values for checked scan entries from
caller-resolved plane, transform-class, and hidden-parity facts.

#### Scenario: Low-frequency limits derive maxLevel
- **GIVEN** checked scan entries and a caller-resolved transform class and plane
- **WHEN** the helper derives `maxLevel`
- **THEN** it applies the AV2 § 5.20.7.27 `get_lf_limits(row, col, txClass,
  plane)` branches
- **AND** it returns luma low-frequency, chroma low-frequency, and non-low
  frequency `maxLevel` values matching the spec constants
- **AND** it can convert those records into the existing quant-pass input shape

#### Scenario: Hidden final scan entry overrides maxLevel
- **GIVEN** hidden parity is active
- **WHEN** the checked scan entry has `c == 0`
- **THEN** the helper derives `NUM_BASE_LEVELS + 1` as `maxLevel`
- **AND** non-final scan entries keep the low-frequency or non-low-frequency
  result

#### Scenario: Runtime decode remains unchanged
- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the max-level derivation helper yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the helper
