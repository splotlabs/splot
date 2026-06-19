## ADDED Requirements

### Requirement: Coefficient FSC sign pass support status

The decoder support model SHALL track `DECODE-COEFF-FSC-SIGN-PASS` as a
distinct crate-private row named `coeff-fsc-sign-pass`. The row SHALL mark only
loaded-but-unwired FSC/IDTX sign symbol sequencing and local `QuantSign[]`
writes as partial support, and SHALL keep runtime `useFsc`, `read_quant`,
nonzero `Quant[]`, dequantization, reconstruction, output, reference refresh,
public APIs, and AVM/dav2d evidence unsupported until separately implemented.

#### Scenario: Matrix records the FSC sign boundary

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-fsc-sign-pass` appears with Feature ID
  `DECODE-COEFF-FSC-SIGN-PASS`
- **AND** it cites AV2 sections 5.20.7.27 and 8.3.2 for FSC sign symbol order
  and selector derivation
- **AND** it names focused FSC sign-pass tests
- **AND** broad runtime `coeffs()`, reconstruction, output, and reference
  support remain partial or unsupported in their existing rows
