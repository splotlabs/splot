# coeff-level-state-write Specification

## Purpose

Define the completed OpenSpec requirements for `coeff-level-state-write`.

## Requirements
### Requirement: Decode coefficient level state writes
The decoder coefficient-loop boundary SHALL provide a crate-private ordinary
non-FSC `Level[]` state-application helper tracked by
`DECODE-COEFF-LEVEL-STATE-WRITE`. The helper SHALL accept a nonzero coefficient
block start, checked scan-walk entries, and decoded base/base-range symbol
records, SHALL write each decoded level to the local transform-block
`Level[row][col]` location from the checked scan entry, and SHALL return the
updated local block state without writing `QuantSign[]`, `Quant[]`, tile context
lines, or reconstruction state.

#### Scenario: Decoded levels are written by checked scan position
- **WHEN** the helper receives a nonzero block start, a matching
  `NonZeroCoeffScanWalk`, and matching `CoeffBaseSymbolRead` records
- **THEN** each read level is written to `Level[row][col]` for the row and column
  carried by its checked scan entry
- **AND** all other local `Level[]` entries remain zero
- **AND** the returned state preserves the decoded nonzero EOB facts for later
  coefficient-loop stages

#### Scenario: Quantization state remains untouched
- **WHEN** decoded base levels are applied
- **THEN** local `QuantSign[]` entries remain zero
- **AND** local `Quant[]` entries remain zero
- **AND** no sign symbols, `read_quant`, dequantization, inverse transform,
  residual add, or reconstruction step is executed

#### Scenario: Mismatched read inputs fail before writes
- **WHEN** the number of decoded level records differs from the checked scan walk
  or any record targets a different scan entry
- **THEN** the helper returns a typed error before writing any local `Level[]`
  entry

#### Scenario: Mismatched block geometry fails before writes
- **WHEN** the checked scan walk targets a row or column outside the consumed
  transform-block state
- **THEN** the helper returns a typed coefficient-state error before writing any
  local `Level[]` entry

#### Scenario: Runtime coefficient decode remains out of scope
- **WHEN** the minimal runtime decode path is exercised after this change
- **THEN** it still does not execute nonzero coefficient level state writes
- **AND** fixture output remains unchanged
