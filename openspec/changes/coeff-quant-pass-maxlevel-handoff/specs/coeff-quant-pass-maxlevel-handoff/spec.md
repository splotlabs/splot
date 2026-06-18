## ADDED Requirements

### Requirement: Quant pass maxLevel handoff
The decoder SHALL provide a crate-private ordinary non-FSC coefficient helper
that derives AV2 § 5.20.7.27 `maxLevel` values and runs the existing quant-pass
composer without requiring per-coefficient `maxLevel` caller inputs.

#### Scenario: Derived maxLevel inputs feed quant pass
- **GIVEN** checked scan entries, local levels, sign summaries, caller-resolved
  plane and transform class, and block-level quant-pass facts
- **WHEN** the helper runs the quant pass
- **THEN** it derives `maxLevel` inputs using the existing § 5.20.7.27
  derivation helper
- **AND** it uses the quant-pass hidden-parity flag for the hidden `c == 0`
  override
- **AND** it delegates literal reads and signed `Quant[]` writes to the existing
  quant-pass composer

#### Scenario: Runtime decode remains unchanged
- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the max-level handoff helper yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the helper
