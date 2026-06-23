# coeff-nonzero-context-commit Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-nonzero-context-commit`.

## Requirements
### Requirement: Nonzero ordinary pass commits coefficient context lines

The decoder SHALL provide a crate-private ordinary non-FSC nonzero coefficient
boundary, tracked by `DECODE-COEFF-NONZERO-CONTEXT-COMMIT`, that runs the
derived-base and derived-sign ordinary coefficient pass, then commits the final
§5.20.7.27 `culLevel` and `dcCategory` values to `TileCoeffContextState`
above/left level and DC context lines. The boundary SHALL remain
loaded-but-unwired and SHALL NOT claim runtime `coeffs()` support.

#### Scenario: Successful pass writes above and left contexts

- **WHEN** the context-commit boundary receives a valid tile coefficient context
  state, nonzero block start, scan, derived-base config, derived-sign config,
  lossless flag, and context-update geometry
- **THEN** it returns the ordinary nonzero pass result
- **AND** it writes the pass result's final `culLevel` to the selected
  `AboveLevelContext` and `LeftLevelContext` ranges
- **AND** it writes the pass result's final `dcCategory` to the selected
  `AboveDcContext` and `LeftDcContext` ranges

#### Scenario: Pass failure does not mutate context lines

- **WHEN** the context-commit boundary fails while running the ordinary pass
  before a final quant-state summary exists
- **THEN** it returns a typed ordinary-pass error
- **AND** the tile coefficient context lines remain unchanged from their
  pre-call values

#### Scenario: Context update failure preserves pre-existing context lines

- **WHEN** the ordinary pass succeeds but the caller-resolved context-update
  plane or geometry is invalid for the tile coefficient context state
- **THEN** the boundary returns a typed ordinary-pass error wrapping the
  coefficient state update error
- **AND** the tile coefficient context lines remain unchanged from their
  pre-call values

#### Scenario: Runtime decode remains unchanged

- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the nonzero context-commit boundary yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the composer
