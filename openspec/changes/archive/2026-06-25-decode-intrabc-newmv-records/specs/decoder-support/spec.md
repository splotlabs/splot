## MODIFIED Requirements

### Requirement: Decoder support matrix records local decoder mission selectable transform progress

The decoder support matrix SHALL document the current local `local-decoder-mission.ivf`
selectable-transform runtime frontier, including the exact unsupported reason,
the spec sections consumed, focused tests, and the behavior explicitly not
claimed.

#### Scenario: IntrABC NEWMV block-vector handoff is tracked

- **WHEN** the local decoder mission probe advances past the previous
  `unsupported_wienerns_lr_selectable_transform_records_intrabc_newmv` stop
- **THEN** `docs/DECODER-SUPPORT-MATRIX.toml` SHALL list the IntrABC
  `assign_mv(0)` / `read_mv()` handoff, the focused tests, and the new
  post-block-vector unsupported reason.
- **AND** it SHALL continue to state that IntrABC prediction/current-frame copy,
  decoded sample population, loop restoration, output, reference refresh,
  AVM/dav2d byte equality, and full local decoder mission decode are not claimed.
