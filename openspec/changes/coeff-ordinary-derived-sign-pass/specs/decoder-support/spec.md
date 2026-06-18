## ADDED Requirements

### Requirement: Coefficient ordinary derived-sign pass support status

The decoder support model SHALL track
`DECODE-COEFF-ORDINARY-DERIVED-SIGN-PASS` as a distinct crate-private partial
decoder boundary named `coeff-ordinary-derived-sign-pass`. The row SHALL mark
only loaded-but-unwired ordinary non-FSC coefficient pass composition with
derived base/level first-pass facts and derived sign sources as partial support,
and SHALL keep runtime nonzero coefficient decode, tile context writes,
dequantization, reconstruction, output, reference refresh, public APIs, and
AVM/dav2d evidence unsupported until separately implemented.

#### Scenario: Matrix records the derived-sign ordinary pass

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-ordinary-derived-sign-pass` appears with Feature ID
  `DECODE-COEFF-ORDINARY-DERIVED-SIGN-PASS`
- **AND** it cites AV2 §5.20.7.27, §5.20.7.28, and §8.3.2 for the base,
  sign-source, sign-read, and quant syntax sequence
- **AND** it names focused tests for explicit/derived equivalence,
  hidden-parity sign derivation, and invalid derived sign selectors without
  quant consumption
- **AND** broad runtime `coeffs()`, reconstruction, output, and reference
  support remain partial or unsupported in their existing rows
