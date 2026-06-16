## ADDED Requirements

### Requirement: Decoder support matrix tracks frame and plane model fuzz coverage

The decoder support matrix SHALL include a row named
`recon-frame-plane-types-fuzz`, tracked by Feature ID `CONF-RECON-FRAME-PLANE-TYPES-FUZZ`,
covering no-panic fuzz coverage for the source-backed `splot-recon`
decoded-frame and plane runtime type validators and accessors.

#### Scenario: frame and plane fuzz row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `recon-frame-plane-types-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/recon_frame_plane_types_bytes.rs` as
  evidence
- **AND** it records bounded frame/plane model generation, typed invalid-case
  coverage, and fuzz target enumeration commands
- **AND** it does not mark byte-consuming decode, reconstruction, output
  scheduling, reference refresh, film grain, metadata MD5 verification,
  resource diagnostics, AVM/dav2d differential testing, or filesystem
  publication as supported
