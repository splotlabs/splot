# coeff-ordinary-derived-sign-pass Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-ordinary-derived-sign-pass`.

## Requirements
### Requirement: Ordinary pass derives sign sources after derived base

The decoder SHALL provide a crate-private ordinary non-FSC coefficient-pass
composition boundary, tracked by `DECODE-COEFF-ORDINARY-DERIVED-SIGN-PASS`,
that uses the state-derived base/level first pass as the source for local
`Level[]`, hidden-parity summary state, and `sumAbs1`, derives
`CoeffSignReadInput` records from those facts and caller-resolved DC context
facts, then runs the existing interleaved sign, `maxLevel`, §5.20.7.28
`read_quant`, and signed `Quant[]` steps. The boundary SHALL remain
loaded-but-unwired and SHALL NOT claim runtime `coeffs()` support.

#### Scenario: Derived sign pass matches explicit-sign composition

- **WHEN** the derived-sign ordinary pass receives the same nonzero block start,
  scan, base derivation config, sign-derivation config, lossless flag, and
  symbol payload as an explicit ordinary pass whose base inputs, sign inputs,
  and quant summary facts are taken from the derived first pass
- **THEN** both pass boundaries return the same base reads, sign reads,
  `read_quant` records, signed `Quant[]` writes, and final local coefficient
  block
- **AND** the derived-sign pass exposes both the derived base selector inputs
  and derived sign inputs for audit and tests

#### Scenario: First-pass hidden parity derives the final sign source

- **WHEN** the first pass makes hidden parity active with positive `sumAbs1`
  for the final `c == 0` carrier
- **THEN** the derived-sign ordinary pass derives a sign input for that final
  entry even when local `Level[0][0]` is zero
- **AND** the later quantized-state write can apply the hidden-parity signed DC
  magnitude without caller-supplied sign inputs

#### Scenario: Invalid derived sign selectors fail before sign and quant syntax

- **WHEN** the derived-sign ordinary pass receives sign-derivation facts that
  produce an out-of-range CDF selector for a derived sign source
- **THEN** it returns a typed ordinary-pass error wrapping the sign-read error
- **AND** no sign CDF row, sign literal, `read_quant` literal, or signed
  `Quant[]` write is consumed after the first pass

#### Scenario: Runtime decode remains unchanged

- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the derived-sign ordinary pass yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the composer
