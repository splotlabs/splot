## ADDED Requirements

### Requirement: Coefficient nonzero context commit support status

The decoder support model SHALL track
`DECODE-COEFF-NONZERO-CONTEXT-COMMIT` as a distinct crate-private partial
decoder boundary named `coeff-nonzero-context-commit`. The row SHALL mark only
loaded-but-unwired ordinary non-FSC nonzero coefficient pass composition with
end-of-`coeffs()` tile coefficient context-line commits as partial support, and
SHALL keep runtime nonzero coefficient decode, dequantization, reconstruction,
output, reference refresh, public APIs, and AVM/dav2d evidence unsupported
until separately implemented.

#### Scenario: Matrix records the nonzero context commit boundary

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-nonzero-context-commit` appears with Feature ID
  `DECODE-COEFF-NONZERO-CONTEXT-COMMIT`
- **AND** it cites AV2 §5.20.7.27 for the end-of-`coeffs()` level/DC context
  update and §5.20.7.28 for the quant syntax whose result feeds that update
- **AND** it names focused tests for successful above/left context writes,
  ordinary-pass failure preserving context state, and invalid context-update
  facts preserving context state
- **AND** broad runtime `coeffs()`, reconstruction, output, and reference
  support remain partial or unsupported in their existing rows
