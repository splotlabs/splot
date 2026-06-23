# coeff-state-context-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-state-context-handoff`.

## Requirements
### Requirement: State-backed ordinary pass uses tile DC contexts before commit

The decoder SHALL provide a crate-private ordinary non-FSC nonzero coefficient
boundary, tracked by `DECODE-COEFF-STATE-CONTEXT-HANDOFF`, that reads
`AboveDcContext[plane]` and `LeftDcContext[plane]` from `TileCoeffContextState`
before running derived sign-source selection, then commits the final
§5.20.7.27 `culLevel` and `dcCategory` values back through that same tile
coefficient context state. The boundary SHALL remain loaded-but-unwired and
SHALL NOT claim runtime `coeffs()` support.

#### Scenario: Successful pass reads seeded DC contexts before writing final context

- **WHEN** the state-backed boundary receives seeded tile coefficient context
  lines, a valid nonzero block start, scan, derived-base facts, sign/source
  facts, lossless flag, and context-update geometry
- **THEN** it returns the ordinary nonzero pass result
- **AND** sign-source derivation observes the pre-call DC context lines
- **AND** the selected above and left level/DC context ranges are updated from
  the final quant-state summary after the pass succeeds

#### Scenario: Pass failure does not mutate context lines

- **WHEN** the state-backed boundary fails while running the ordinary pass
  before a final quant-state summary exists
- **THEN** it returns a typed ordinary-pass error
- **AND** the tile coefficient context lines remain unchanged from their
  pre-call values

#### Scenario: Context update failure preserves pre-existing context lines

- **WHEN** the ordinary pass succeeds but the state-backed boundary receives an
  invalid context-update plane or geometry for the tile coefficient context
  state
- **THEN** the boundary returns a typed ordinary-pass error wrapping the
  coefficient state update error
- **AND** the tile coefficient context lines remain unchanged from their
  pre-call values

#### Scenario: Runtime decode remains unchanged

- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the state-backed ordinary nonzero coefficient
  boundary yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the composer
