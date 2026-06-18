## MODIFIED Requirements

### Requirement: Decoder crate dependency direction

The repository SHALL enforce the approved decoder/reconstruction/encoder
dependency boundary through `cargo xtask check-dependency-direction`.
`splot-recon` SHALL depend on no other `splot-*` crate except `splot-tables`.
`splot-decode` MAY depend on `splot-core`, `splot-parallel`, and `splot-recon`
when implementation code needs those crates. `splot-cli` MAY depend on
`splot-decode` for library-owned decoder diagnostics and future CLI decode
integration. `splot-encode` MAY depend on `splot-core`, `splot-parallel`, and
`splot-recon` after the `encoder-recon-dependency` change.

#### Scenario: Approved decoder and encoder graph is accepted

- **WHEN** `cargo xtask check-dependency-direction` runs
- **THEN** the allow-list includes `splot-recon`, `splot-decode`, and the direct
  `splot-encode -> splot-recon` edge
- **AND** any internal dependency outside the approved graph is rejected.

#### Scenario: Coverage threshold stays validator-scoped

- **WHEN** the workspace gains or reuses `splot-recon` and `splot-decode`
- **THEN** local and CI coverage threshold commands keep gating
  `crates/splot-validate` line coverage only
- **AND** the scaffold or integration crates do not accidentally join the
  validator coverage threshold.
