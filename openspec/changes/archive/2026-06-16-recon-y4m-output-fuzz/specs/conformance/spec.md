## ADDED Requirements

### Requirement: reconstruction Y4M output serialization fuzz target

The repository SHALL provide a cargo-fuzz target named
`recon_y4m_output_bytes`, tracked by Feature ID
`CONF-RECON-Y4M-OUTPUT-FUZZ`, that builds bounded valid
`splot-recon` decoded frames from arbitrary bytes and serializes them through
`Y4mWriter` without filesystem, network, subprocess, AVM, dav2d, or ffmpeg
dependencies.

#### Scenario: structured decoded frames serialize without panics

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it normalizes those bytes into small valid decoded-frame inputs for
  supported Y4M bit-depth and pixel-format combinations
- **AND** it calls `Y4mWriter` to serialize stream headers, frame headers, and
  visible frame payloads
- **AND** success or failure is represented by the public typed return path
  without panicking

#### Scenario: output serialization remains bounded

- **WHEN** fuzz input requests larger dimensions, extra frames, stride padding,
  or more sample data than the target permits
- **THEN** the target clamps those values to fixed CI-safe bounds before
  allocating sample buffers or serializing output

#### Scenario: smoke automation enumerates the target

- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `recon_y4m_output_bytes` is included in target execution without
  hardcoding the executable target list in CI workflow files
