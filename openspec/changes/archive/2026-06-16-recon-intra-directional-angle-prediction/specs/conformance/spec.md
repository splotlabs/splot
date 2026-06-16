## ADDED Requirements

### Requirement: Directional Angle Intra Prediction Fuzz Metadata
The conformance metadata SHALL record that `recon_intra_prediction_bytes`
exercises the source-backed one-sided directional-angle primitive with bounded,
structured, self-contained inputs. The fuzz coverage SHALL remain a typed-error
and no-panic property over reconstruction primitives, not a broad AV2 bitstream
or runtime decode conformance claim.

#### Scenario: Fuzz coverage remains self-contained
- **WHEN** `cargo xtask check-fuzz-targets` and
  `cargo check --manifest-path fuzz/Cargo.toml --bins --locked` validate the
  fuzz target list
- **THEN** `recon_intra_prediction_bytes` covers one-sided directional-angle
  direct primitive inputs without requiring AVM, dav2d, filesystem output,
  network access, subprocesses, generated external corpora, or runtime
  `splot decode`
