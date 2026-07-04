## MODIFIED Requirements

### Requirement: local decoder mission selectable narrow-record live frontier evidence

Decoder support tracking SHALL record that the local decoder mission live probe advances
past `unsupported_wienerns_lr_live_transform_record_mrl_mode` after the active
MRL record handoff and reaches
`unsupported_wienerns_lr_live_transform_record_fsc_mode`. The support rows SHALL
record the new structured unsupported frontier, proof commands, and explicit
non-goals for decoded samples, loop-restoration filtering/output, reference
refresh, AVM/dav2d byte equality, and successful local decoder mission decode.

#### Scenario: Local local decoder mission probe reaches next post-MRL frontier

- **WHEN** the local `local-decoder-mission.ivf` probe runs after the active MRL record handoff
- **THEN** decoder support status records
  `unsupported_wienerns_lr_live_transform_record_fsc_mode`
- **AND** the previous active-MRL diagnostic is no longer the live local decoder mission
  frontier

### Requirement: local decoder mission active intra tool support row

The decoder support model SHALL track
`DECODE-ACTIVE-INTRA-TOOL-FRONTIER` as a distinct partial local decoder mission row
named `active-intra-tool-frontier`. The row SHALL describe that
selectable Wiener NS LR transform-record derivation consumes active MRL syntax,
retains `UsesMrls` metadata for LR tx-skip record derivation, uses retained
neighbour state for MRL CDF contexts, and relaxes broad sequence-level
intra/transform tool gates plus parsed CCSO filter state into active-use or
later filter/output diagnostics, while remaining fail-closed before decoded
frame samples, loop-restoration filtering/output, reference refresh, AVM/dav2d
byte equality, or successful local decoder mission decode.

#### Scenario: Matrix evidence records active MRL record handoff

- **WHEN** decoder support status is validated
- **THEN** `active-intra-tool-frontier` appears with Feature ID
  `DECODE-ACTIVE-INTRA-TOOL-FRONTIER`
- **AND** the row cites AV2 §5.20.5.3, §5.20.5.5, §5.20.7.27, §8.3.2, and §9.3
- **AND** it lists focused MRL CDF/context/mode-info tests plus the local decoder mission
  runtime probe
- **AND** it does not claim decoded frame samples, loop-restoration filtering,
  output, reference refresh, AVM/dav2d byte equality, or successful local decoder mission
  decode
