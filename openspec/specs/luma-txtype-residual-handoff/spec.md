# local decoder mission Luma TxType Residual Handoff Specification

## Purpose
Define the fail-closed local decoder mission Wiener NS LR handoff that retains resolved luma
transform types for syntax-only residual parsing while decoded sample
reconstruction remains unsupported.

## Requirements

### Requirement: local decoder mission luma transform-type residual handoff

The decoder SHALL track `DECODE-LUMA-TXTYPE-RESIDUAL-HANDOFF` as a
partial runtime prerequisite for the local decoder mission Wiener NS LR path. When selectable
transform-record derivation reaches nonzero luma residual syntax whose active
AV2 §5.20.8.2 / §5.20.8.3 transform-type path resolves to a non-DCT
`PlaneTxType`, the syntax-only LR tx-skip handoff SHALL retain that resolved
`PlaneTxType`, SHALL derive the ordinary coefficient scan and contexts from its
transform class, and SHALL remain fail-closed before reconstruction or output.

#### Scenario: LR handoff admits active non-DCT luma transform type

- **WHEN** the local decoder mission Wiener NS LR transform-record path reaches nonzero
  luma residual syntax
- **AND** active luma transform-type syntax maps to a valid non-DCT `PlaneTxType`
- **AND** the caller selected the LR tx-skip record handoff policy
- **THEN** the decoder consumes the supported syntax and uses the resolved
  `PlaneTxType` for scan/class derivation
- **AND** it advances to the next structured unsupported-feature frontier

#### Scenario: Reconstruction-safe callers still reject

- **WHEN** a reconstruction-safe residual caller reaches the same active non-DCT
  luma transform-type syntax
- **THEN** the decoder returns a structured `decode/unsupported-feature`
  diagnostic before decoded samples, residual addition, filtering, output, or
  reference refresh are produced

#### Scenario: No successful local decoder mission decode claim

- **WHEN** the luma transform-type residual handoff succeeds
- **THEN** the decoder SHALL NOT claim inverse transforms, residual addition,
  decoded output, loop-restoration filtering, reference refresh, AVM/dav2d byte
  equality, or successful local decoder mission decode
