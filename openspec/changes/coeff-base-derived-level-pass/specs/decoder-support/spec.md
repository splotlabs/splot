## ADDED Requirements

### Requirement: Coefficient base derived level pass support row
The decoder support model SHALL track `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS` as
a distinct crate-private row named `coeff-base-derived-level-pass`. The row SHALL
mark only the loaded ordinary non-FSC first pass from nonzero block start through
local `Level[]` writes and first-pass TCQ/parity summary as partial
coefficient-loop support.

#### Scenario: Matrix records state-derived first-pass support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-base-derived-level-pass` appears with Feature ID
  `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`
- **AND** it cites AV2 § 5.20.7.27 and § 8.3.2 as syntax/selector evidence
- **AND** it names tests for state-derived selector ordering, immediate level
  writes, TCQ selector-state updates, parity summary updates, and preflight
  failure preservation
- **AND** it does not claim runtime `coeffs()` integration, runtime scan-table or
  transform fact derivation, sign-source selection, quant reads, tile
  context-line commits, dequantization, inverse transform, residual add,
  reconstruction, reference refresh, AVM/dav2d evidence, public APIs, or full
  decoder conformance

#### Scenario: Generated docs remain honest
- **WHEN** feature status, spec coverage, decoder support, and decoder
  conformance coverage status documents are regenerated
- **THEN** the new row remains partial until a later runtime `coeffs()`
  integration change proves reachable decode behavior
