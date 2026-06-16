## ADDED Requirements

### Requirement: runtime hash byte fuzz target

The repository SHALL provide a cargo-fuzz target named
`decode_runtime_hash_bytes`, tracked by Feature ID
`CONF-DECODE-RUNTIME-HASH-FUZZ`, that calls
`DecodeContext::decode_hash_report_bytes` with finite decode limits and no
external decoder, filesystem, or network dependency.

#### Scenario: arbitrary bytes return typed results

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it passes fuzz-selected bytes to
  `DecodeContext::decode_hash_report_bytes`
- **AND** success or failure is represented by the public typed return path
  without panicking
- **AND** the target does not invoke AVM, dav2d, ffmpeg, filesystem I/O, or the
  network

#### Scenario: fixture mutations exercise the minimal runtime success path

- **GIVEN** the committed `syn-flat-intra-64x64-minimal.ivf` fixture
- **WHEN** fuzz input selects the fixture-mutation mode
- **THEN** the target applies bounded deterministic mutations before calling
  `DecodeContext::decode_hash_report_bytes`
- **AND** an unmutated or still-supported input that decodes successfully is
  checked for the current minimal hash-report shape without claiming broad AV2
  decode support

#### Scenario: smoke automation enumerates the target

- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `decode_runtime_hash_bytes` is included in target execution without
  hardcoding the executable target list in CI workflow files
- **AND** CI corpus seeding MAY include target-specific prefix seeds when a
  target consumes control bytes before the bitstream payload
