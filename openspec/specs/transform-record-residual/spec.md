# local decoder mission Transform-Record Residual Specification

## Purpose
Define the fail-closed local decoder mission Wiener NS LR handoff that consumes supported
transform-record residual syntax while decoded sample reconstruction remains
unsupported.

## Requirements

### Requirement: LR Transform-Record Residual Handoff
The decoder SHALL consume the live local decoder mission Wiener NS LR transform-record
residual syntax with AV2 §5.20.7.24, §5.20.7.25, §5.20.7.27, and §5.20.7.30
derived transform sizes, scan order, and chroma/CCTX transform-block ordering
when the caller selects the syntax-only LR handoff policy. The handoff SHALL NOT
claim decoded samples, inverse transforms, residual addition, loop restoration,
output, reference refresh, AVM/dav2d equality, or successful local decoder mission decode.

#### Scenario: Live residual syntax advances to a structured frontier
- **WHEN** the local `local-decoder-mission.ivf` stream reaches the active Wiener NS LR
  transform-record residual path
- **THEN** the decoder consumes the supported residual syntax needed for LR
  tx-skip handoff
- **AND** the next stop is a structured unsupported-feature diagnostic for the
  next unimplemented decoder frontier

#### Scenario: Invalid scan geometry remains rejected
- **WHEN** a coefficient block derives an EOB larger than the AV2 §5.20.7.30 scan
  length for its resolved transform size
- **THEN** the decoder rejects the block with a structured residual parse error
- **AND** it does not clamp, truncate, or fabricate coefficient positions

#### Scenario: Reconstruction-safe callers remain fail-closed
- **WHEN** a reconstruction-safe residual caller reaches the same active
  transform-record residual syntax before output support exists
- **THEN** the decoder returns a structured unsupported-feature diagnostic
  before producing decoded samples or output
