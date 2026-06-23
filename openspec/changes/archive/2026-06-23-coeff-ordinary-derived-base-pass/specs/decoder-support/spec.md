## ADDED Requirements

### Requirement: Coefficient ordinary derived-base pass support status

The decoder support model SHALL track
`DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS` as a distinct crate-private partial
decoder boundary named `coeff-ordinary-derived-base-pass`. The row SHALL mark
only loaded-but-unwired ordinary non-FSC coefficient pass composition with
derived base/level first-pass facts as partial support, and SHALL keep runtime
nonzero coefficient decode, tile context writes, dequantization, reconstruction,
output, reference refresh, public APIs, and AVM/dav2d evidence unsupported until
separately implemented.

#### Scenario: Matrix records the derived-base ordinary pass

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-ordinary-derived-base-pass` appears with Feature ID
  `DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS`
- **AND** it cites AV2 §5.20.7.27 and §5.20.7.28 for the first-pass and
  second-pass coefficient syntax sequence
- **AND** it names focused tests for explicit/derived equivalence, hidden
  parity summary handoff, and first-pass preflight failure
- **AND** broad runtime `coeffs()`, reconstruction, output, and reference
  support remain partial or unsupported in their existing rows
