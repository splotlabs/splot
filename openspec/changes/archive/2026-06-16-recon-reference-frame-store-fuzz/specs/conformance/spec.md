## ADDED Requirements

### Requirement: reconstruction reference-frame store fuzz target

The repository SHALL provide a cargo-fuzz target named
`recon_reference_frame_store_bytes`, tracked by Feature ID
`CONF-RECON-REFERENCE-FRAME-STORE-FUZZ`, that drives the public `splot-recon`
`ReferenceSlot` and `ReferenceFrameStore<F>` storage APIs with bounded arbitrary
operation sequences and no filesystem, network, subprocess, AVM, dav2d, or
ffmpeg dependencies.

#### Scenario: bounded operation sequences exercise store APIs without panics

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it normalizes those bytes into bounded capacity, slot, payload, and
  operation inputs
- **AND** it calls public `ReferenceSlot` and `ReferenceFrameStore<F>` APIs for
  construction, containment, lookup, insertion, removal, clearing, occupancy,
  and entry iteration
- **AND** success or failure is represented by the public typed return path
  without panicking

#### Scenario: reference store state matches an oracle

- **WHEN** fuzz operations mutate a valid reference-frame store
- **THEN** occupied count, emptiness, slot contents, replacement returns, removal
  returns, and ascending entry order match a bounded oracle model after each
  checkpoint

#### Scenario: reference store fuzzing remains bounded

- **WHEN** fuzz input requests invalid capacities, invalid slots, or longer
  operation streams than the target permits
- **THEN** the target clamps operation count and payload size to fixed CI-safe
  bounds while preserving invalid-capacity and invalid-slot coverage through
  typed errors

#### Scenario: smoke automation enumerates the target

- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `recon_reference_frame_store_bytes` is included in target execution
  without hardcoding the executable target list in CI workflow files
