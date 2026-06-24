## ADDED Requirements

### Requirement: ac0ej3 transform-record residual tracking
Decoder support tracking SHALL record the ac0ej3 transform-record residual
handoff as a partial row update that advances the live stream frontier without
claiming inverse transforms, residual addition, loop-restoration filtering,
decoded output, reference refresh, AVM/dav2d byte equality, or successful
ac0ej3 decode.

#### Scenario: Live frontier evidence is recorded
- **WHEN** the local `ac0ej3.ivf` probe advances after the transform-record
  residual handoff
- **THEN** the decoder support matrix records the new unsupported frontier,
  feature ID, proof commands, and explicit non-goals
