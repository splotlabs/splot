# local decoder mission Chroma CCTX Handoff Specification

## Purpose

Track the local decoder mission decoder mission boundary where chroma CCTX and non-DCT
chroma residual syntax is consumed for Wiener NS LR tx-skip record derivation
without claiming chroma reconstruction or output.

## Requirements

### Requirement: LR Chroma CCTX Metadata Handoff

The decoder SHALL consume active AV2 §5.20.7.27 chroma CCTX type syntax in the
local decoder mission Wiener NS LR tx-skip record path only when the caller selects the
syntax-only handoff policy, and SHALL treat the decoded value as metadata that
is not used for reconstruction or output.

#### Scenario: CCTX metadata is admitted for LR tx-skip syntax

- **WHEN** a U-plane nonzero chroma residual in the LR tx-skip record path
  requires `cctx_type`
- **THEN** the decoder consumes the symbol and continues deriving
  transform-record syntax without claiming decoded samples or output support

#### Scenario: Reconstruction-safe callers still reject CCTX

- **WHEN** a reconstruction-safe residual caller sees active CCTX type syntax
- **THEN** the decoder returns a structured `decode/unsupported-feature`
  diagnostic before output

### Requirement: LR Chroma Transform-Set Handoff

The decoder SHALL allow the local decoder mission Wiener NS LR tx-skip record handoff to
consume intra chroma coefficient syntax when AV2 §5.20.8.3 does not force
`DCT_DCT`, while reconstruction-safe residual callers SHALL continue to reject
the same surface.

#### Scenario: Chroma non-DCT transform set is consumed for LR handoff

- **WHEN** an intra chroma residual in the LR tx-skip record path has a
  non-DCT-only transform set
- **THEN** the decoder derives the real chroma transform type through the
  ordinary coefficient branch and records the residual syntax needed for LR
  tx-skip state

#### Scenario: Reconstruction-safe callers still fail closed

- **WHEN** a reconstruction-safe residual caller sees the same chroma non-DCT
  transform-set surface
- **THEN** the decoder rejects it with a structured unsupported-feature
  diagnostic before output
