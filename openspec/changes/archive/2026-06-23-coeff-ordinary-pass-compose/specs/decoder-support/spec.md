## ADDED Requirements

### Requirement: Coefficient ordinary pass composition support row
The decoder support model SHALL track `DECODE-COEFF-ORDINARY-PASS-COMPOSE` as a
distinct crate-private row named `coeff-ordinary-pass-compose`. The row SHALL
mark only the loaded ordinary non-FSC composition from nonzero block start
through signed `Quant[]` writes as partial coefficient-loop support.

#### Scenario: Matrix records narrow ordinary pass support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-ordinary-pass-compose` appears with Feature ID
  `DECODE-COEFF-ORDINARY-PASS-COMPOSE`
- **AND** it cites AV2 § 5.20.7.27, § 5.20.7.28, and § 8.2.5 as syntax
  evidence
- **AND** it names tests for successful composition and failed-boundary
  preservation
- **AND** it does not claim runtime evolving base selector derivation,
  post-level sign-source selection, runtime scan-table or transform fact
  derivation, tile context-line commits, dequantization, inverse transform,
  residual add, reconstruction, reference refresh, AVM/dav2d evidence, public
  APIs, or full decoder conformance

#### Scenario: Generated docs remain honest
- **WHEN** feature status, spec coverage, decoder support, and decoder
  conformance coverage status documents are regenerated
- **THEN** the new row remains partial until a later runtime `coeffs()`
  integration change proves reachable decode behavior
