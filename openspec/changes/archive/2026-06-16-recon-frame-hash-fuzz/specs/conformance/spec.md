## ADDED Requirements

### Requirement: reconstruction frame hash serialization fuzz target

The repository SHALL provide a cargo-fuzz target named
`recon_frame_hash_bytes`, tracked by Feature ID
`CONF-RECON-FRAME-HASH-FUZZ`, that builds bounded valid `splot-recon` decoded
frames from arbitrary bytes and exercises `DecodedFrameHashInput` byte
serialization and digest computation without filesystem, network, subprocess,
AVM, dav2d, or ffmpeg dependencies.

#### Scenario: structured decoded frames hash without panics

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it normalizes those bytes into small valid decoded-frame inputs for
  supported bit-depth, sample-storage, pixel-format, crop, padding, and stride
  combinations
- **AND** it calls `DecodedFrameHashInput::byte_len`, `write_to`, and
  `compute_hash`
- **AND** success or writer failure is represented by the public typed return
  path without panicking

#### Scenario: hash input ignores non-visible frame storage and metadata

- **WHEN** two generated decoded frames have identical visible samples but
  different non-visible padding samples and output indices
- **THEN** the fuzz target verifies their emitted hash-input bytes and computed
  digests remain equal

#### Scenario: frame hash fuzzing remains bounded

- **WHEN** fuzz input requests larger dimensions, crop origins, storage padding,
  stride padding, or writer budgets than the target permits
- **THEN** the target clamps those values to fixed CI-safe bounds before
  allocating sample buffers, serializing output, or exercising failing writers

#### Scenario: smoke automation enumerates the target

- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `recon_frame_hash_bytes` is included in target execution without
  hardcoding the executable target list in CI workflow files
