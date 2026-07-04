## ADDED Requirements

### Requirement: local decoder mission transform-record residual tracking
Decoder support tracking SHALL record the local decoder mission transform-record residual
handoff as a partial row update that advances the live stream frontier without
claiming inverse transforms, residual addition, loop-restoration filtering,
decoded output, reference refresh, AVM/dav2d byte equality, or successful
local decoder mission decode.

#### Scenario: Live frontier evidence is recorded
- **WHEN** the local `local-decoder-mission.ivf` probe advances after the transform-record
  residual handoff
- **THEN** the decoder support matrix records the new unsupported frontier,
  feature ID, proof commands, and explicit non-goals
