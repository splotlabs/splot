# encoder-tools delta: writer-validate-crosstool

## ADDED Requirements

### Requirement: writer output is validator-conformant

A bitstream that the `splot-core` writer produces SHALL pass the `splot-validate` validator with zero
error-severity diagnostics (cross-tool agreement). This SHALL be demonstrated by re-emitting each
committed conformant fixture that consists only of writable OBU types (temporal delimiter, padding,
metadata) through the complete-OBU writer and validating the re-emission: the re-emitted stream SHALL
be byte-exact to the canonical original and SHALL be reported as conformant
(`ValidationReport::is_conformant`).

#### Scenario: re-emitting a conformant fixture stays conformant

- **WHEN** a committed conformant fixture of writable OBU types is parsed, re-emitted through the
  writer, and validated
- **THEN** every OBU SHALL round-trip, the re-emission SHALL be byte-exact to the original, and the
  validator SHALL report zero error diagnostics.
