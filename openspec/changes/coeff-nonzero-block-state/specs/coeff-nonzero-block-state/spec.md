## ADDED Requirements

### Requirement: Initialize nonzero coefficient block state before EOB syntax
The decoder coefficient-loop boundary SHALL allocate zero-initialized local
coefficient block state for the nonzero `coeffs()` branch before consuming the
nonzero EOB syntax.

#### Scenario: Nonzero branch returns zeroed block state and EOB read
- **WHEN** the caller selects the nonzero coefficient branch with valid
  transform geometry and valid EOB context facts
- **THEN** the handoff returns the nonzero EOB read result
- **AND** it returns a zero-initialized local `Level[]`, `QuantSign[]`, and
  `Quant[]` block sized from the caller-resolved transform geometry
- **AND** coefficient context state remains unchanged

#### Scenario: Invalid nonzero geometry consumes no symbols
- **WHEN** the caller selects the nonzero branch with invalid transform geometry
- **THEN** the handoff returns the existing coefficient-state error
- **AND** tile CDF rows and symbol-decoder counters remain unchanged

#### Scenario: Invalid EOB facts preserve mutable state
- **WHEN** the caller selects the nonzero branch with valid transform geometry
  but invalid transform log2 facts for EOB selector derivation
- **THEN** the handoff returns the typed transform-log2 error
- **AND** coefficient context state, tile CDF rows, and symbol-decoder counters
  remain unchanged
