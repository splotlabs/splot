## ADDED Requirements

### Requirement: Decoder support matrix tracks Y4M serialization fuzz coverage

The decoder support matrix SHALL include a row named
`recon-y4m-output-fuzz`, tracked by Feature ID
`CONF-RECON-Y4M-OUTPUT-FUZZ`, covering no-panic fuzz coverage for
source-backed `splot-recon` Y4M serialization over bounded caller-supplied
decoded frames.

#### Scenario: Y4M fuzz row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `recon-y4m-output-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/recon_y4m_output_bytes.rs` as evidence
- **AND** it records bounded structured frame generation and fuzz target
  enumeration commands
- **AND** it does not mark broad runtime decode, byte-consuming runtime Y4M
  decode, raw output, AVM/dav2d differential testing, or filesystem
  publication as supported
