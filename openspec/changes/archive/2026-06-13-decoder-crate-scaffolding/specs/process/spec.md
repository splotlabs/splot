## ADDED Requirements

### Requirement: Decoder crate dependency direction

The repository SHALL enforce the approved decoder/reconstruction dependency
boundary through `cargo xtask check-dependency-direction`. `splot-recon` SHALL
depend on no other `splot-*` crate. `splot-decode` MAY depend on `splot-core`
and `splot-recon` when implementation code needs those crates. `splot-cli` MAY
depend on `splot-decode` only when a future CLI integration change wires decode
behavior. `splot-encode` MAY depend on `splot-recon` only through a future
encoder/reconstruction API change.

#### Scenario: Approved decoder graph is accepted

- **WHEN** `cargo xtask check-dependency-direction` runs
- **THEN** the allow-list includes `splot-recon` and `splot-decode`
- **AND** any internal dependency outside the approved graph is rejected

#### Scenario: Coverage threshold stays validator-scoped

- **WHEN** the workspace gains `splot-recon` and `splot-decode`
- **THEN** local and CI coverage threshold commands keep gating
  `crates/splot-validate` line coverage only
- **AND** the new scaffold crates do not accidentally join the validator
  coverage threshold
