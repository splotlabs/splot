## ADDED Requirements

### Requirement: Coefficient FSC level pass support status

The decoder support model SHALL track `DECODE-COEFF-FSC-LEVEL-PASS` as a
distinct crate-private row named `coeff-fsc-level-pass`. The row SHALL mark only
loaded-but-unwired FSC/IDTX level symbol sequencing and local `Level[]` writes
as partial support, and SHALL keep runtime `useFsc`, IDTX sign reads,
`read_quant`, nonzero `QuantSign[]` and `Quant[]`, dequantization,
reconstruction, output, reference refresh, public APIs, and AVM/dav2d evidence
unsupported until separately implemented.

#### Scenario: Matrix records the FSC level boundary

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-fsc-level-pass` appears with Feature ID
  `DECODE-COEFF-FSC-LEVEL-PASS`
- **AND** it cites AV2 sections 5.20.7.27 and 8.3.2 for FSC level symbol order
  and selector derivation
- **AND** it names focused FSC level-pass tests
- **AND** broad runtime `coeffs()`, reconstruction, output, and reference
  support remain partial or unsupported in their existing rows
