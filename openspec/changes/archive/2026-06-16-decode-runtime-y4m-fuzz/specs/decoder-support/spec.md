## ADDED Requirements

### Requirement: Decoder support matrix tracks runtime Y4M fuzz coverage
The decoder support matrix SHALL include a row named
`decode-runtime-y4m-fuzz`, tracked by Feature ID
`CONF-DECODE-RUNTIME-Y4M-FUZZ`, covering no-panic fuzz coverage for the current
minimal-tier `DecodeContext::decode_y4m_bytes` byte-consuming API over bounded
raw input and minimal-fixture mutation inputs.

#### Scenario: runtime Y4M fuzz row is scoped and test-backed
- **GIVEN** the generated decoder support status
- **WHEN** the `decode-runtime-y4m-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/decode_runtime_y4m_bytes.rs` as
  evidence
- **AND** it records fuzz target enumeration, focused runtime Y4M tests, and a
  local nightly fuzz smoke command
- **AND** it does not mark broad AV2 runtime decode, full Y4M output
  conformance, CLI filesystem publication, raw output, hash report output,
  post-film-grain output, show-existing/flush scheduling, reference refresh,
  metadata MD5 verification, AVM/dav2d differential testing, or support beyond
  the committed minimal IVF tier as supported
