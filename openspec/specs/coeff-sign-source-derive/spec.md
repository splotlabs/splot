# coeff-sign-source-derive Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-sign-source-derive`.

## Requirements
### Requirement: Derive coefficient sign sources

The decoder SHALL provide a crate-private ordinary non-FSC coefficient
sign-source derivation helper tracked by `DECODE-COEFF-SIGN-SOURCE-DERIVE`.
The helper SHALL accept local `Level[]` state, a checked scan walk, and
caller-resolved block facts for plane, plane type, transform class, hidden
parity, `sumAbs1`, and above/left DC context lines, and SHALL return
`CoeffSignReadInput` records selecting `dc_sign`, `dc_sign_horz_vert`,
`sign_bit`, or no sign source without consuming symbols, mutating CDF rows,
writing coefficient state, or claiming runtime decode support.

#### Scenario: Luma DC sign uses dc_sign context

- **WHEN** the checked entry is row 0, column 0, plane 0 and either local
  `Level[0][0]` is nonzero or hidden parity requires a final sign
- **THEN** the derived sign source is `dc_sign`
- **AND** the selector uses the caller-provided coefficient CDF q-context, plane
  type, hidden group, and the §8.3.2 `dc_sign_ctx` computed from above/left DC
  context lines and block coordinates

#### Scenario: Luma axis signs use horizontal or vertical DC sign row

- **WHEN** the transform class is horizontal, the checked entry has column 0,
  and plane is 0
- **THEN** the derived sign source is `dc_sign_horz_vert` with context 0
- **AND** when the transform class is vertical, the checked entry has row 0,
  and plane is 0, the helper derives the same syntax and context 0

#### Scenario: Generic signs use raw sign bits

- **WHEN** the entry requires a sign but is not luma DC and does not match the
  luma horizontal or vertical axis branches
- **THEN** the derived sign source is `sign_bit`
- **AND** chroma coefficients use `sign_bit` even at row 0, column 0

#### Scenario: Zero entries without hidden parity skip signs

- **WHEN** local `Level[row][col]` is zero and hidden parity does not require
  the final `c == 0` sign
- **THEN** the derived sign source is `None`
- **AND** the returned input still preserves the matching checked scan entry

#### Scenario: State lookup errors are typed and non-consuming

- **WHEN** a checked scan entry names a row or column outside the local
  transform block
- **THEN** the helper returns a typed coefficient state error
- **AND** no symbol decoder state, CDF row, coefficient state, or runtime decode
  output is consumed or mutated by the derivation helper

#### Scenario: Runtime decode remains unchanged

- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the sign-source derivation helper yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the composer
