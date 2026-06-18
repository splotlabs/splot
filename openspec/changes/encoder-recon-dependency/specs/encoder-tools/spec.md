## MODIFIED Requirements

### Requirement: Closed-loop reconstruction reuse is gated

The encoder program SHALL treat `splot-recon` as available lower-level
reconstruction building blocks through a direct `splot-encode -> splot-recon`
dependency. That dependency edge SHALL NOT be treated as an integrated encoder
reconstruction loop until later input-view, closed-loop, and proof changes land.

#### Scenario: recon dependency is not public encode integration

- **WHEN** the `encoder-recon-dependency` change is reviewed
- **THEN** `splot-encode` depends on `splot-recon` only as an approved lower-level
  crate boundary
- **AND** no encoder public API reports successful encoded output because of this
  dependency alone.

#### Scenario: future closed-loop work uses the approved boundary

- **WHEN** later encoder frame-input or closed-loop reconstruction work starts
- **THEN** it may design against the approved `splot-recon` dependency
- **AND** it must still provide its own Feature IDs, tests, and matrix proof.
