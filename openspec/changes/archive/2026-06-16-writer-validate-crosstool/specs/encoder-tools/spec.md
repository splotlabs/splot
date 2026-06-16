# encoder-tools delta: writer-validate-crosstool

## ADDED Requirements

### Requirement: re-emitting a conformant stream stays validator-conformant

The `splot-core` writer SHALL re-emit an already validator-conformant bitstream (a parser-produced
stream of writable OBU types) as a stream that itself passes the `splot-validate` validator with zero
error-severity diagnostics (cross-tool agreement). This is NOT a claim that every writer output is
conformant: `write_complete_obu` faithfully serializes any encodable model, including one the
validator would reject (e.g. a parser-producible header that fails a § 6 conformance rule); the writer
reproduces its input, it does not validate it, so conformance is guaranteed only for the re-emission of
an already-conformant input. The guarantee SHALL be demonstrated by re-emitting each committed
conformant fixture that consists only of writable OBU types (temporal delimiter, padding, metadata)
through the complete-OBU writer and validating the re-emission: the re-emitted stream SHALL be
byte-exact to the canonical original and SHALL be reported as conformant
(`ValidationReport::is_conformant`).

#### Scenario: re-emitting a conformant fixture stays conformant

- **WHEN** a committed conformant fixture of writable OBU types is parsed, re-emitted through the
  writer, and validated
- **THEN** every OBU SHALL round-trip, the re-emission SHALL be byte-exact to the original, and the
  validator SHALL report zero error diagnostics.
