## ADDED Requirements

### Requirement: Decoder support matrix tracks runtime hash fuzz coverage

The decoder support matrix SHALL include a row named
`decode-runtime-hash-fuzz`, tracked by Feature ID
`CONF-DECODE-RUNTIME-HASH-FUZZ`, covering no-panic fuzz coverage for the current
minimal `DecodeContext::decode_hash_report_bytes` byte-consuming API.

#### Scenario: runtime hash fuzz row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `decode-runtime-hash-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/decode_runtime_hash_bytes.rs` as
  evidence
- **AND** it records finite-limit behavior and the commands that compile or
  enumerate the fuzz target
- **AND** it does not mark broad runtime decode, full tile syntax, full
  reconstruction, AVM/dav2d differential testing, or Y4M/raw output fuzzing as
  supported
