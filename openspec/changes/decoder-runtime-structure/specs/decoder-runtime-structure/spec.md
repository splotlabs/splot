# decoder-runtime-structure delta: decoder-runtime-structure

This change advances Feature ID `DECODE-RUNTIME-STRUCTURE`.

## ADDED Requirements

### Requirement: Decoder production modules use domain names

Production `splot-decode` code SHALL be organized by decoder responsibility
rather than under `runtime_minimal` or `runtime_minimal_recon`.

#### Scenario: production modules are domain based

- **WHEN** the decoder crate module tree is inspected
- **THEN** production decode modules use names such as `pipeline`, `bitstream`,
  `prediction`, `residual`, `reference`, `filters`, `output`, `support`, and
  `tile`
- **AND** no production module path is named `runtime_minimal` or
  `runtime_minimal_recon`

### Requirement: Decode output behavior is preserved

The structural refactor SHALL preserve existing supported hash, raw, and Y4M
decode behavior.

#### Scenario: committed supported outputs still decode

- **WHEN** the existing decoder and CLI decode tests run
- **THEN** supported committed fixtures produce the same hash, raw, and Y4M
  outputs as before the refactor

### Requirement: Decoder docs identify the extension point

Decoder documentation SHALL explain the module map and where to add new decoder
features without relying on historical runtime names.

#### Scenario: contributor finds decoder ownership

- **WHEN** a contributor reads `docs/DECODER-ARCHITECTURE.md`
- **THEN** the document describes bitstream planning, tile/entropy decode,
  prediction, residual reconstruction, filters/restoration, reference state,
  output handling, and support gates
- **AND** migration history is kept in a decision record rather than source
  comments

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
