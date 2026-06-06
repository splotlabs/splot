# process delta: encoder-reference-gate

## ADDED Requirements

### Requirement: encoder reference gate

The repository SHALL require contributors to consult the reference notes before
encoder work uses rav1e, SVT-AV1, or another AV1 implementation as research input,
and to confirm the change does not copy AV1 syntax, constants, tables, comments,
prose, or decoder-visible semantics into `splot`. Tracked by
`DOC-ENCODER-REFERENCE-GATE`.

#### Scenario: encoder change uses an AV1 implementation as research input

- **WHEN** a contributor opens an encoder-facing change informed by rav1e, SVT-AV1,
  or another AV1 implementation
- **THEN** the PR explains the source as research context only and records any
  decoder-visible AV2 mapping work separately
