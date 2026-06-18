# coeff-eob-derived-symbol-read Specification

## Purpose

Define the completed OpenSpec requirements for `coeff-eob-derived-symbol-read`.

## Requirements
### Requirement: Read EOB syntax from derived context facts
The decoder coefficient-loop boundary SHALL read nonzero coefficient EOB syntax
from caller-resolved transform log2 dimensions, plane/inter state, and
coefficient CDF quantization context by deriving the active EOB CDF selector
facts before consuming symbols.

#### Scenario: Derived read matches explicit selector read
- **WHEN** the caller provides valid transform log2 dimensions and plane/inter
  facts that map to an explicit `EobPtSize` and `eobCtx`
- **THEN** the derived reader returns the same EOB symbol-read result and consumes
  the same symbol state as the explicit-selector EOB reader

#### Scenario: Invalid transform facts consume no mutable state
- **WHEN** the caller provides a width or height log2 value below the AV2
  transform minimum
- **THEN** the derived reader returns a typed coefficient-loop context error
  before mutating tile CDF rows or consuming symbol bits

#### Scenario: Symbol-reader errors still propagate
- **WHEN** selector derivation succeeds but the selected EOB CDF row or literal
  read fails
- **THEN** the derived reader returns the existing EOB symbol-read or literal-read
  error variant
