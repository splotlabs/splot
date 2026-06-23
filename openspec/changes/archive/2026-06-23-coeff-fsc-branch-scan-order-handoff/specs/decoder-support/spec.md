## ADDED Requirements

### Requirement: Decoder support tracks FSC branch scan-order handoff
The decoder support matrix and decoder conformance coverage tooling SHALL track `DECODE-COEFF-FSC-BRANCH-SCAN-ORDER` as a partial loaded-but-unwired decoder-support row linked to AV2 § 5.20.7.27, § 5.20.7.30, § 8.3.2, and the generated § 9.2 transform-size tables.

#### Scenario: Tracking rows stay synchronized
- **WHEN** the FSC branch scan-order handoff is implemented
- **THEN** `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, `xtask/src/decoder_conformance_coverage.rs`, and the generated status docs MUST include the new Feature ID/support row without marking runtime `coeffs()` or broad decode support complete
