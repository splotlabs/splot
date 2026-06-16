## ADDED Requirements

### Requirement: reconstruction intra prediction fuzz target

The repository SHALL provide a cargo-fuzz target named
`recon_intra_prediction_bytes`, tracked by Feature ID
`CONF-RECON-INTRA-PREDICTION-FUZZ`, that builds bounded structured inputs for
existing `splot-recon` DC, PAETH, smooth, and current-frame workspace intra
prediction APIs without filesystem, network, subprocess, AVM, dav2d, or ffmpeg
dependencies.

#### Scenario: structured intra prediction inputs return typed results

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it normalizes those bytes into bounded valid or intentionally typed-
  error inputs for the existing intra prediction and workspace APIs
- **AND** it calls the public prediction/workspace APIs for DC, PAETH, smooth,
  and source-backed workspace prediction cases
- **AND** success or failure is represented by the public typed return path
  without panicking

#### Scenario: intra fuzzing remains bounded

- **WHEN** fuzz input requests larger block sizes, strides, workspace planes,
  sample buffers, or operation counts than the target permits
- **THEN** the target clamps those values to fixed CI-safe bounds before
  allocating buffers or invoking prediction code

#### Scenario: smoke automation enumerates the target

- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `recon_intra_prediction_bytes` is included in target execution
  without hardcoding the executable target list in CI workflow files
