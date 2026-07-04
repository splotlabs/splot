## ADDED Requirements

### Requirement: local decoder mission selectable narrow-record live frontier evidence

Decoder support tracking SHALL record that the local decoder mission live probe advances
past `unsupported_wienerns_lr_selectable_transform_records_empty_transform`
after the selectable narrow-record handoff. The support rows SHALL record the
new structured unsupported frontier, proof commands, and explicit non-goals for
decoded samples, loop-restoration filtering/output, reference refresh,
AVM/dav2d byte equality, and successful local decoder mission decode.

#### Scenario: Local local decoder mission probe reaches active MRL frontier

- **WHEN** the local `local-decoder-mission.ivf` probe runs after the selectable narrow-record
  handoff
- **THEN** decoder support status records the new active-MRL unsupported
  frontier
- **AND** the previous empty-transform diagnostic is no longer the live local decoder mission
  frontier
