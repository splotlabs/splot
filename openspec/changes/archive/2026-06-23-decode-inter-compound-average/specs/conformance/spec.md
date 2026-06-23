## ADDED Requirements

### Requirement: Compound-average fixture has portable reference evidence
The conformance metadata SHALL include a committed three-frame fixture for
`DECODE-INTER-COMPOUND-AVERAGE` whose bytes are listed in the conformance
manifest and whose local-reference evidence records matching raw-output digests
from `avmdec` and `dav2d`. The evidence SHALL remain portable metadata and SHALL
NOT require external decoders during CI.

#### Scenario: fixture metadata is hermetic
- **WHEN** `cargo xtask conformance` validates the committed IVF corpus
- **THEN** the compound-average fixture is listed exactly once with its expected
  clean validation outcome

#### Scenario: local-reference evidence is metadata only
- **WHEN** `cargo xtask check-decoder-support` validates local-reference evidence
- **THEN** the compound-average evidence entry references the fixture, the
  `DECODE-INTER-COMPOUND-AVERAGE` Feature ID, and the `inter-compound-average`
  decoder-support row
- **AND** the check does not invoke `avmdec`, `dav2d`, a network connection, or
  any external decoder
