## ADDED Requirements

### Requirement: Decoder support matrix tracks reference-frame store fuzz coverage

The decoder support matrix SHALL include a row named
`recon-reference-frame-store-fuzz`, tracked by Feature ID
`CONF-RECON-REFERENCE-FRAME-STORE-FUZZ`, covering no-panic fuzz coverage for the
source-backed `splot-recon` reference-frame store storage API over bounded
operation sequences.

#### Scenario: reference-frame store fuzz row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `recon-reference-frame-store-fuzz` row is rendered
- **THEN** it records
  `fuzz/fuzz_targets/recon_reference_frame_store_bytes.rs` as evidence
- **AND** it records bounded operation-sequence generation and fuzz target
  enumeration commands
- **AND** it does not mark byte-consuming decode, AV2 reference refresh
  semantics, `RefValid`, `refresh_frame_flags`, output scheduling,
  motion-field storage, resource diagnostics, AVM/dav2d differential testing,
  or filesystem publication as supported
