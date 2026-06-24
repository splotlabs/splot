## ADDED Requirements

### Requirement: ac0ej3 chroma CCTX handoff tracking
Decoder support tracking SHALL record the ac0ej3 chroma CCTX metadata syntax handoff as a partial row that advances the live stream frontier without claiming CCTX reconstruction, chroma output, or successful ac0ej3 decode.

#### Scenario: Live frontier evidence is recorded
- **WHEN** the local `ac0ej3.ivf` probe advances after the chroma CCTX metadata handoff
- **THEN** the decoder support matrix records the new unsupported frontier, feature ID, proof commands, and explicit non-goals
