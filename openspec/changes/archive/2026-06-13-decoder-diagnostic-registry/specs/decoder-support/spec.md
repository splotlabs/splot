## ADDED Requirements

### Requirement: canonical decoder diagnostic registry

Decoder diagnostics emitted by `splot decode` SHALL be documented in
`docs/DECODER-DIAGNOSTICS.md` with stable field names `rule_id`, `severity`,
`spec_section`, `matrix_row`, `feature_id`, `message`, and `remediation` when
applicable. The `spec_section` field SHALL cite an AV2 section when the
diagnostic is tied to AV2 decoding behavior, and the decoder support matrix
SHALL link emitted decoder diagnostics to support rows. Tracked by
`DOC-DECODER-DIAGNOSTICS`.

#### Scenario: decode diagnostic is emitted

- **WHEN** `splot decode` emits a `decode/*` diagnostic
- **THEN** the rule ID is present in `docs/DECODER-DIAGNOSTICS.md`
- **AND** the diagnostic is linked to a row in
  `docs/DECODER-SUPPORT-MATRIX.toml`

#### Scenario: unsupported decode entry point is documented

- **WHEN** `splot decode` reports the current unsupported entry point
- **THEN** `decode/unsupported-feature` is documented with severity `Error`,
  AV2 §7.1, `CLI-DECODE`, and matrix row `cli-decode-entrypoint`
