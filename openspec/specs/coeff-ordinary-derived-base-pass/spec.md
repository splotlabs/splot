# coeff-ordinary-derived-base-pass Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-ordinary-derived-base-pass`.

## Requirements
### Requirement: Ordinary pass derives base first-pass state

The decoder SHALL provide a crate-private ordinary non-FSC coefficient-pass
composition boundary, tracked by `DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS`,
that uses the state-derived base/level first pass as the source for
base/base-range symbol reads, local `Level[]` writes, hidden-parity summary
state, and `sumAbs1` before running the existing interleaved sign, `maxLevel`,
`read_quant`, and signed `Quant[]` steps. The boundary SHALL remain
loaded-but-unwired and SHALL NOT claim runtime `coeffs()` support.

#### Scenario: Derived base pass matches explicit-base composition

- **WHEN** the ordinary pass receives the same nonzero block start, scan, sign
  inputs, transform facts, TCQ flag, lossless flag, and symbol payload as an
  explicit-base ordinary pass whose base inputs and quant summary facts are
  taken from the derived first pass
- **THEN** both pass boundaries return the same base reads, sign reads,
  `read_quant` records, signed `Quant[]` writes, and final local coefficient
  block
- **AND** the derived-base pass exposes the first pass's derived base selector
  inputs for audit and tests

#### Scenario: First-pass hidden parity feeds second-pass quant

- **WHEN** the first pass reads enough nonzero pre-DC coefficients to make
  hidden parity active before `c == 0`
- **THEN** the derived-base ordinary pass uses the first-pass `isHidden` and
  `sumAbs1` facts for its second-pass sign and `read_quant` handling
- **AND** callers cannot override those facts through the derived-base input

#### Scenario: Invalid first-pass facts fail before consumption

- **WHEN** the derived-base ordinary pass receives inconsistent first-pass facts
  such as parity hiding and TCQ enabled together
- **THEN** it returns a typed ordinary-pass error wrapping the first-pass error
- **AND** no symbol decoder state or tile CDF state is consumed

#### Scenario: Runtime decode remains unchanged

- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the derived-base ordinary pass yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the composer
