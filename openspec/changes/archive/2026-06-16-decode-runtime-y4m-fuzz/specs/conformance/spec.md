## ADDED Requirements

### Requirement: runtime Y4M decode fuzz target
The repository SHALL provide a cargo-fuzz target named
`decode_runtime_y4m_bytes`, tracked by Feature ID
`CONF-DECODE-RUNTIME-Y4M-FUZZ`, that drives the existing
`DecodeContext::decode_y4m_bytes` byte-consuming API with bounded in-memory
inputs and writers without filesystem, network, subprocess, AVM, dav2d, or
ffmpeg dependencies.

#### Scenario: runtime Y4M byte inputs return typed results
- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it feeds either bounded raw input bytes or bounded mutations of the
  committed minimal IVF fixture into `DecodeContext::decode_y4m_bytes`
- **AND** successful decoding writes a complete in-memory Y4M stream for the
  current minimal runtime tier
- **AND** unsupported, malformed, resource-limit, or output failures are
  represented by public typed `DecodeError` returns without panicking

#### Scenario: runtime Y4M fuzzing remains bounded
- **WHEN** fuzz input requests larger raw input, mutation counts, decode work,
  tile payloads, decoded frames, reference storage, or output bytes than the
  target permits
- **THEN** the target clamps those values to fixed CI-safe limits before
  invoking the runtime Y4M API

#### Scenario: runtime Y4M writer behavior is in memory
- **WHEN** the fuzz target exercises successful output or caller-writer failure
  paths
- **THEN** it uses bounded in-memory writers
- **AND** it never creates, opens, renames, fsyncs, or deletes filesystem output
  paths

#### Scenario: smoke automation enumerates the target
- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `decode_runtime_y4m_bytes` is included in target execution without
  hardcoding the executable target list in CI workflow files
