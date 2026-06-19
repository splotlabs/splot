## ADDED Requirements

### Requirement: FSC coefficient branch handoff

The decoder SHALL provide a crate-private loaded-but-unwired FSC/IDTX nonzero
coefficient branch handoff that composes the existing nonzero EOB start, checked
FSC scan walk, FSC level pass, and FSC quant/context-commit stages for AV2
§5.20.7.27.

#### Scenario: FSC branch matches explicit staged pipeline

- **GIVEN** caller-resolved nonzero EOB facts, `segEob`, scan order, FSC
  level-pass config, and luma context-commit geometry
- **WHEN** the FSC branch handoff runs successfully
- **THEN** it returns the same FSC pass result as the explicit staged pipeline
  of nonzero EOB start, checked FSC scan walk, level pass, and quant/context
  commit
- **AND** it commits the final `culLevel` and `dcCategory` to the same tile
  context ranges

#### Scenario: All-zero routing is rejected without mutation

- **GIVEN** an all-zero coefficient branch input
- **WHEN** the FSC branch handoff is called
- **THEN** it returns a typed FSC branch routing error
- **AND** it preserves tile coefficient context state, CDF state, and symbol
  decoder position

#### Scenario: Invalid FSC scan facts are rejected before FSC symbol reads

- **GIVEN** a decoded nonzero EOB start and a caller-resolved `segEob` or scan
  order that cannot cover the FSC `bob..segEob` window
- **WHEN** the FSC branch handoff derives the checked scan walk
- **THEN** it returns the scan-walk error before FSC level/sign/quant symbol
  reads

#### Scenario: Chroma routing is rejected before EOB consumption

- **GIVEN** a nonzero FSC branch input whose context-commit plane is not luma
- **WHEN** the FSC branch handoff is called
- **THEN** it returns a typed non-luma routing error before EOB, FSC level,
  sign, or quant symbols are consumed
