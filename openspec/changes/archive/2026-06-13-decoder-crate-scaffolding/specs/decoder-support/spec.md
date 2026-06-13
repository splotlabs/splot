## ADDED Requirements

### Requirement: Decoder crate scaffolding

The repository SHALL provide approved workspace crate scaffolds for future
decoder and reconstruction work. The scaffold SHALL be tracked by Feature ID
`INFRA-DECODER-CRATE-SCAFFOLDING`. `splot-recon` SHALL be the future home for
pixel buffers, deterministic decoded-frame hash primitives, reconstruction
primitives, and reference-frame storage. `splot-decode` SHALL be the future home
for the decoder driver that combines `splot-core` parsing with `splot-recon`
state. The scaffold SHALL NOT claim runtime AV2 decode, reconstruction,
deterministic hash, Y4M output, bit-exact output, or AV2 conformance support.

#### Scenario: Scaffolds build without runtime API claims

- **WHEN** the workspace is checked
- **THEN** `crates/splot-recon` and `crates/splot-decode` build as library
  crates with crate-level documentation and workspace lint inheritance
- **AND** they do not expose public placeholder reconstruction or decode APIs
  merely to prove the crates exist

#### Scenario: Decode behavior remains unsupported

- **WHEN** `splot decode` runs after the scaffold is added
- **THEN** it keeps the existing structured unsupported diagnostic behavior
- **AND** no input bytes are decoded, no output is written, and no external
  decoder is located or invoked

#### Scenario: Crate support status stays honest

- **WHEN** decoder support status is rendered
- **THEN** the scaffold row is represented as repository infrastructure
- **AND** codec decode stages remain `todo`, `partial`, `blocked`, or
  `unsupported-intentional` according to their own proof, not because the crates
  exist
