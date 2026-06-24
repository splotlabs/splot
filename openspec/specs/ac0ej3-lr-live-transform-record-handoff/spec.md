# ac0ej3 LR Live Transform-Record Handoff Specification

## Purpose
Define the fail-closed ac0ej3 Wiener NS loop-restoration frontier that hands
tile-derived transform facts into live `LrTxSkip` storage before decoded sample
population is supported.

## Requirements

### Requirement: ac0ej3 LR Live Transform-Record Handoff

The decoder SHALL track `DECODE-AC0EJ3-LR-LIVE-TRANSFORM-RECORD-HANDOFF` as a
partial Wiener NS LR prerequisite. For fixed-largest transform blocks, the
runtime handoff SHALL derive luma `WienerNsLrTxSkipTransformRecord` values from
parsed tile transform facts, SHALL derive a complete retained
`WienerNsLrTxSkipGrid` with the AV2 §5.20.7.27
`skip_flag || (eob == 0)` rule, and SHALL populate the live LR storage shell
from that grid before stopping at the next unsupported decoded-sample
prerequisite.

#### Scenario: Fixed-largest records populate live tx-skip storage

- **WHEN** the runtime has parsed fixed-largest luma transform facts covering
  the retained `LrTxSkip` grid
- **THEN** it derives a complete `WienerNsLrTxSkipGrid`
- **AND** it populates the live LR storage shell with those tile-derived values
- **AND** it remains fail-closed before live `CurrFrame` and `CdefFrame` sample
  population

#### Scenario: Selectable transform mode remains fail-closed

- **WHEN** the local ac0ej3 mission stream reaches active luma Wiener NS LR
- **AND** its key frame uses `TX_MODE_SELECT`
- **THEN** the runtime returns `decode/unsupported-feature`
- **AND** the diagnostic cites the transform-record handoff feature and matrix
  row
- **AND** it does not fabricate `LrTxSkip`, `CurrFrame`, or `CdefFrame` values

#### Scenario: No successful ac0ej3 decode claim

- **WHEN** the transform-record handoff frontier is reached
- **THEN** the decoder SHALL NOT claim selectable transform partition parsing,
  `FilterClass` retention, `SubclassLookup`, loop-restoration filtering/output,
  reference refresh, AVM/dav2d byte equality, or successful ac0ej3 decode
