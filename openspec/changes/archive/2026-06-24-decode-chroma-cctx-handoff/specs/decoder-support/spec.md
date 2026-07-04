## ADDED Requirements

### Requirement: local decoder mission chroma CCTX handoff tracking
Decoder support tracking SHALL record the local decoder mission chroma CCTX metadata syntax handoff as a partial row that advances the live stream frontier without claiming CCTX reconstruction, chroma output, or successful local decoder mission decode.

#### Scenario: Live frontier evidence is recorded
- **WHEN** the local `local-decoder-mission.ivf` probe advances after the chroma CCTX metadata handoff
- **THEN** the decoder support matrix records the new unsupported frontier, feature ID, proof commands, and explicit non-goals
