## ADDED Requirements

### Requirement: Coefficient quant pass maxLevel handoff support row
The decoder support model SHALL track
`DECODE-COEFF-QUANT-PASS-MAXLEVEL-HANDOFF` as a distinct crate-private row named
`coeff-quant-pass-maxlevel-handoff`. The row SHALL mark only the loaded
ordinary non-FSC handoff from max-level derivation into quant-pass composition
as partial coefficient-loop support.

#### Scenario: Matrix records narrow handoff support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-quant-pass-maxlevel-handoff` appears with Feature ID
  `DECODE-COEFF-QUANT-PASS-MAXLEVEL-HANDOFF`
- **AND** it cites AV2 § 5.20.7.27 and § 5.20.7.28 as syntax evidence
- **AND** it names tests for low-frequency max-level handoff, hidden final-entry
  max-level handoff, and bad-fact no-consumption behavior
- **AND** it does not claim runtime coefficient-loop execution, selector or
  scan-table derivation, dequantization, inverse transform, residual add,
  reconstruction, reference refresh, AVM/dav2d evidence, public APIs, or full
  decoder conformance

#### Scenario: Generated docs remain honest
- **WHEN** feature status, spec coverage, decoder support, and decoder
  conformance coverage status documents are regenerated
- **THEN** the new row remains partial until a later runtime `coeffs()`
  integration change proves reachable decode behavior
