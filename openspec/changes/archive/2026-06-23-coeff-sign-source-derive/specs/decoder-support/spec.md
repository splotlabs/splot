## ADDED Requirements

### Requirement: Coefficient sign-source derivation support status

The decoder support model SHALL track `DECODE-COEFF-SIGN-SOURCE-DERIVE` as a
distinct crate-private partial decoder boundary named
`coeff-sign-source-derive`. The row SHALL mark only ordinary non-FSC
coefficient sign-source derivation from local `Level[]`, hidden parity,
transform class, plane, and DC contexts as partial support, and SHALL keep
runtime nonzero coefficient decode, tile context writes, dequantization,
reconstruction, output, reference refresh, public APIs, and AVM/dav2d evidence
unsupported until separately implemented.

#### Scenario: Matrix records sign-source derivation

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-sign-source-derive` appears with Feature ID
  `DECODE-COEFF-SIGN-SOURCE-DERIVE`
- **AND** it cites AV2 §5.20.7.27 and §8.3.2 for the sign source branch and
  `dc_sign` CDF context selection
- **AND** it names focused tests for luma DC, horizontal-axis, vertical-axis,
  generic sign-bit, skipped zero-level, hidden-parity, and state-error behavior
- **AND** broad runtime `coeffs()`, reconstruction, output, and reference
  support remain partial or unsupported in their existing rows
