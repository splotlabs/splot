## ADDED Requirements

### Requirement: Coefficient ordinary branch handoff support row
The decoder support model SHALL track `DECODE-COEFF-ORDINARY-BRANCH-HANDOFF`
as a distinct partial `splot-decode` row named
`coeff-ordinary-branch-handoff`. The row SHALL cite AV2 § 5.20.7.27,
§ 5.20.7.28, § 8.2.5, and § 8.3.2, SHALL record focused tests for all-zero
preservation, nonzero ordinary handoff success, and failure state preservation,
and SHALL keep full runtime `coeffs()`, transform syntax derivation,
dequantization, reconstruction, output, reference refresh, AVM/dav2d evidence,
and public APIs out of scope.

#### Scenario: Matrix records narrow branch support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `coeff-ordinary-branch-handoff` appears with Feature ID
  `DECODE-COEFF-ORDINARY-BRANCH-HANDOFF`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim dequantization, inverse transform, residual add,
  reconstruction, output, reference refresh, or AVM/dav2d invocation

#### Scenario: Coverage tracks the new coefficient handoff
- **WHEN** decoder conformance coverage is generated
- **THEN** the tile group / payload syntax coverage and symbol/CDF process
  coverage include row `coeff-ordinary-branch-handoff` and Feature ID
  `DECODE-COEFF-ORDINARY-BRANCH-HANDOFF`
- **AND** broader tile payload and symbol/CDF coverage remain partial
