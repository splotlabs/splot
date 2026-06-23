# coeff-ordinary-branch-plane-type-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-ordinary-branch-plane-type-handoff`.

## Requirements
### Requirement: Handoff ordinary branch plane to plane type
The decoder coefficient-loop boundary SHALL provide a crate-private ordinary
branch handoff, tracked by `DECODE-COEFF-ORDINARY-BRANCH-PLANE-TYPE-HANDOFF`,
that accepts caller-resolved `plane` for the nonzero ordinary coefficient path,
derives AV2 section 5.20.7.27 `ptype` as `plane > 0`, and delegates to the
existing state-backed `PlaneTxType` ordinary branch. The handoff SHALL preserve
the existing all-zero branch behavior and SHALL NOT implement `compute_tx_type`,
derive scan order, consume additional symbols beyond the delegated branch, mutate
extra CDF rows, dequantize, reconstruct, or expose a public API.

#### Scenario: Nonzero branch derives plane type before delegation
- **WHEN** the caller supplies a nonzero ordinary branch input with a
  caller-resolved `plane`
- **THEN** the handoff derives the matching `plane_type` and returns the same
  branch result as the existing explicit `plane_type` ordinary branch input

#### Scenario: Chroma planes map to chroma plane type
- **WHEN** the caller supplies a nonzero ordinary branch input for plane 1 or
  plane 2
- **THEN** the handoff derives `plane_type` 1 before selecting state-backed sign
  contexts and committing coefficient context lines

#### Scenario: All-zero branch is unchanged
- **WHEN** the caller supplies an all-zero ordinary branch input
- **THEN** the handoff delegates to the existing all-zero coefficient state path
  without requiring or deriving a nonzero state-context `plane_type`
