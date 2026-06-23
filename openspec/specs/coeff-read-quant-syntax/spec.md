# coeff-read-quant-syntax Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-read-quant-syntax`.

## Requirements
### Requirement: Decode coefficient read-quant syntax
The decoder coefficient-loop boundary SHALL provide a crate-private ordinary
non-FSC AV2 § 5.20.7.28 `read_quant` syntax helper tracked by
`DECODE-COEFF-READ-QUANT-SYNTAX`. The helper SHALL accept caller-resolved
`level`, `pos`, `isHidden`, `maxLevel`, `hrLevelAvg`, and `allowTcq` facts,
SHALL consume only the literal bits reached by the § 5.20.7.28 q-length,
Golomb-length, and coefficient-remainder branches, and SHALL return the decoded
`quant` plus updated `hrLevelAvg` without mutating coefficient state, CDF rows,
tile context lines, or runtime decode output.

#### Scenario: Quant below threshold consumes no bits
- **WHEN** `level` is below `maxLevel - allowTcq`
- **THEN** the helper returns `quant == level`
- **AND** it returns the input `hrLevelAvg`
- **AND** it does not consume q-length, Golomb-length, or coefficient-remainder
  literal bits

#### Scenario: Finite q-length path reads the coefficient remainder
- **WHEN** the q-length loop observes a one bit before `cMax`
- **THEN** the helper sets `length = m`
- **AND** it reads exactly `length` coefficient-remainder bits
- **AND** it adds the decoded remainder expression to `quant`
- **AND** it updates `hrLevelAvg` using the § 5.20.7.28 expression

#### Scenario: Golomb extension path reads until a terminator
- **WHEN** the q-length loop reaches `cMax`
- **THEN** the helper reads Golomb-length bits until the first one bit
- **AND** it computes `length += k`, the extended base value, and the
  coefficient remainder with checked arithmetic
- **AND** it returns typed errors instead of panicking if the reached length or
  arithmetic exceeds the helper's local bounds

#### Scenario: Hidden DC and TCQ facts affect the result
- **WHEN** `pos == 0`, `isHidden` is true, and the parser reaches the extended
  quant path
- **THEN** the helper applies `lvlShift = 1` to `predLevel` and `hrLevelAvg`
  update
- **AND** when `allowTcq` is true, it adds `x << 1` to `quant`

#### Scenario: Runtime coefficient decode remains out of scope
- **WHEN** the minimal runtime decode path is exercised after this change
- **THEN** it still does not execute runtime nonzero `read_quant` syntax
- **AND** it does not write nonzero `Quant[]`, dequantize, transform, add
  residuals, reconstruct pixels, or change fixture output
