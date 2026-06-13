## MODIFIED Requirements

### Requirement: Structured decode unsupported diagnostics
Unsupported decoder features SHALL be represented in docs and matrix rows as
structured diagnostics with a stable rule id, severity, optional spec section,
matrix row id, human-readable message, and remediation. The `splot decode`
CLI entry point SHALL emit `decode/unsupported-feature` with severity `Error`,
spec section `7.1`, matrix row `cli-decode-entrypoint`, and Feature ID
`CLI-DECODE` until a supported decoder path replaces the intentional
unsupported implementation.

#### Scenario: Unsupported feature is documented
- **WHEN** a matrix row identifies an unsupported AV2 tool
- **THEN** the row links the unsupported behavior to a stable diagnostic code or
  planned diagnostic code and a spec section where applicable

#### Scenario: Decode command emits text diagnostic
- **WHEN** `splot decode <input> -o <output>` is run before decode support is
  implemented
- **THEN** it exits with code `1`
- **AND** stderr contains diagnostic rule id `decode/unsupported-feature`,
  severity `Error`, spec section `7.1`, matrix row `cli-decode-entrypoint`,
  and Feature ID `CLI-DECODE`
- **AND** no AVM, dav2d, ffmpeg, or external decoder is located or invoked

#### Scenario: Decode command emits JSON diagnostic
- **WHEN** `splot decode --json <input> -o <output>` is run before decode support
  is implemented
- **THEN** it exits with code `1`
- **AND** stdout is a machine-readable diagnostic object containing
  `rule_id = "decode/unsupported-feature"`, `severity = "Error"`,
  `spec_section = "7.1"`, `matrix_row = "cli-decode-entrypoint"`, and
  `feature_id = "CLI-DECODE"`
- **AND** stderr remains empty unless an operational error occurs

#### Scenario: Decode command avoids file I/O while unsupported
- **WHEN** `splot decode <missing-input> -o <output>` is run before decode
  support is implemented
- **THEN** it exits with code `1`
- **AND** it emits `decode/unsupported-feature`
- **AND** it does not create the missing input path or output path
