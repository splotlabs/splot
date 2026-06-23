## ADDED Requirements

### Requirement: Coefficient base Ph CDF row support status

The decoder support model SHALL track
`DECODE-COEFF-BASE-PH-CDF-ROW` as a distinct crate-private row named
`coeff-base-ph-cdf-row`. The row SHALL mark only parity-hidden coefficient base
CDF row loading, selection, lifecycle handling, and loaded-but-unwired
state-derived first-pass consumption as partial support, and SHALL keep runtime
nonzero coefficient decode, tile context writes, dequantization, reconstruction,
output, reference refresh, public APIs, and AVM/dav2d evidence unsupported until
separately implemented.

#### Scenario: Matrix records the Ph CDF row boundary

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-base-ph-cdf-row` appears with Feature ID
  `DECODE-COEFF-BASE-PH-CDF-ROW`
- **AND** it cites AV2 §8.3.2 and §9.3 for CDF selection and generated default
  row evidence
- **AND** it names focused CDF row and first-pass hidden-parity tests
- **AND** broad runtime `coeffs()`, reconstruction, output, and reference
  support remain partial or unsupported in their existing rows
