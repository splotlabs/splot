# coeff-eob-branch-handoff Specification

## Purpose

Define the completed OpenSpec requirements for `coeff-eob-branch-handoff`.

## Requirements
### Requirement: Dispatch coefficient EOB branches after all-zero selection
The decoder coefficient-loop boundary SHALL dispatch the decoded `all_zero`
branch to either all-zero coefficient context state application or nonzero EOB
syntax reading from caller-resolved transform and plane/inter facts.

#### Scenario: All-zero branch applies state without consuming symbols
- **WHEN** the caller selects the all-zero branch with caller-resolved transform
  geometry
- **THEN** the handoff applies the all-zero coefficient-block state effects
- **AND** tile CDF rows and symbol-decoder counters remain unchanged

#### Scenario: Nonzero branch reads EOB without coefficient state mutation
- **WHEN** the caller selects the nonzero branch with valid transform log2
  dimensions, plane/inter facts, and coefficient CDF quantization context
- **THEN** the handoff reads the same EOB syntax as the derived EOB reader
- **AND** coefficient context state remains unchanged

#### Scenario: Invalid nonzero selector facts preserve mutable state
- **WHEN** the caller selects the nonzero branch with an invalid transform log2
  dimension
- **THEN** the handoff returns the typed transform-log2 error
- **AND** coefficient context state, tile CDF rows, and symbol-decoder counters
  remain unchanged
