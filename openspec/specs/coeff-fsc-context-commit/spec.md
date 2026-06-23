# coeff-fsc-context-commit Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-fsc-context-commit`.

## Requirements
### Requirement: FSC coefficient context commit wrapper

The decoder SHALL provide a crate-private loaded-but-unwired FSC/IDTX wrapper
that commits tile coefficient context lines after a successful FSC quant pass.

#### Scenario: FSC pass commits context lines after quant state

- **GIVEN** a completed FSC level pass, checked `0..segEob` scan entries,
  caller-resolved FSC level-pass config, and caller-resolved context commit
  plane and 4x4 geometry
- **WHEN** the wrapper runs the FSC quant pass successfully
- **THEN** it writes the pass final `culLevel` to the selected
  `AboveLevelContext` and `LeftLevelContext` ranges
- **AND** it writes the pass final `dcCategory` to the matching
  `AboveDcContext` and `LeftDcContext` ranges
- **AND** it returns the same local FSC pass result that the non-committing
  helper would return

#### Scenario: Failed FSC pass preserves context state

- **GIVEN** a static FSC pass error before second-loop symbol consumption
- **WHEN** the context-commit wrapper returns the error
- **THEN** tile coefficient context lines are unchanged

#### Scenario: Failed context update preserves context state

- **GIVEN** a successful FSC quant pass and invalid caller-resolved context
  update facts
- **WHEN** the context-commit wrapper returns the update error
- **THEN** tile coefficient context lines are unchanged
