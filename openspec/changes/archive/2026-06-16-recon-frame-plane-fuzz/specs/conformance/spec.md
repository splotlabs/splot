## ADDED Requirements

### Requirement: reconstruction frame and plane model fuzz target

The repository SHALL provide a cargo-fuzz target named
`recon_frame_plane_types_bytes`, tracked by Feature ID
`CONF-RECON-FRAME-PLANE-TYPES-FUZZ`, that drives the public `splot-recon`
decoded-frame and plane model APIs with bounded arbitrary inputs and no
filesystem, network, subprocess, AVM, dav2d, or ffmpeg dependencies.

#### Scenario: bounded inputs exercise frame and plane APIs without panics

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it normalizes those bytes into bounded bit-depth, pixel-format,
  geometry, crop, stride, backing-buffer, sample, and plane-presence inputs
- **AND** it calls public `splot-recon` constructors and accessors for
  `BitDepth`, `PixelFormat`, `Plane<T>`, `FramePlanes<T>`,
  `DecodedFrameInfo`, `DecodedFrame<T>`, borrowed views, and `SharedFrame`
- **AND** success or failure is represented by the public typed return path
  without panicking

#### Scenario: valid decoded frames preserve public invariants

- **WHEN** the fuzz target derives a bounded valid decoded-frame model
- **THEN** decoded-frame metadata, plane presence, visible sizes, visible row
  contents, borrowed `PlaneRef`/`FrameRef` metadata, and explicit shared-frame
  handle counts match the normalized model

#### Scenario: invalid frame and plane cases stay typed and bounded

- **WHEN** fuzz input requests invalid idc values, unaligned crops, too-small
  strides, mismatched backing lengths, missing or unexpected chroma planes,
  mismatched visible sizes, unsupported sample storage, or out-of-range samples
- **THEN** the target keeps allocations within fixed CI-safe bounds and accepts
  only public typed `ReconError` returns for rejected cases

#### Scenario: smoke automation enumerates the target

- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `recon_frame_plane_types_bytes` is included in target execution without
  hardcoding the executable target list in CI workflow files
