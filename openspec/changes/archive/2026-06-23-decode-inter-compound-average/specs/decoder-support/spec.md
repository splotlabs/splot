## ADDED Requirements

### Requirement: Decoder support tracks compound-average subset
The decoder support model SHALL include a partial row for
`DECODE-INTER-COMPOUND-AVERAGE` named `inter-compound-average`. The row SHALL
describe the fixture-proven two-reference equal-weight compound-average subset,
its `decode/unsupported-feature` gates, self-contained tests, and local-reference
evidence pointer, while keeping broad compound inter decode partial or
unsupported.

#### Scenario: support row validates
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  metadata
- **THEN** the `inter-compound-average` row exists with Feature ID
  `DECODE-INTER-COMPOUND-AVERAGE`
- **AND** the row records tests and the local-reference evidence entry for the
  committed fixture

#### Scenario: status does not overclaim
- **WHEN** decoder support status is generated
- **THEN** broad compound, masked, CWP, optical-flow/refine-MV, temporal MV,
  residual compound, and full AV2 decoder conformance remain partial or
  unsupported until separately implemented and proven
