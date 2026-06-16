## ADDED Requirements

### Requirement: Decoder support matrix tracks frame hash fuzz coverage

The decoder support matrix SHALL include a row named
`recon-frame-hash-fuzz`, tracked by Feature ID
`CONF-RECON-FRAME-HASH-FUZZ`, covering no-panic fuzz coverage for source-backed
`splot-recon` decoded-frame hash input serialization and digest computation over
bounded caller-supplied decoded frames.

#### Scenario: frame hash fuzz row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `recon-frame-hash-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/recon_frame_hash_bytes.rs` as evidence
- **AND** it records bounded structured frame generation and fuzz target
  enumeration commands
- **AND** it does not mark broad runtime decode, AV2 decoded-frame-hash metadata
  verification, output ordering, film grain, reference refresh, AVM/dav2d
  differential testing, or filesystem publication as supported
