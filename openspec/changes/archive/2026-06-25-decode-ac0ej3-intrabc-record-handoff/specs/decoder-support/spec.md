## ADDED Requirements

### Requirement: ac0ej3 IntrABC transform-record support row

The decoder support model SHALL record the ac0ej3 IntrABC mode-info handoff
under `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS`. The support row SHALL
describe that the local ac0ej3 Wiener NS LR selectable-transform path consumes
the observed AV2 §5.20.5.3 `use_intrabc = 1` and bounded §5.20.5.4
`read_intrabc_info()` syntax into retained transform-record metadata, and SHALL
continue to mark decoded samples, IntrABC prediction, loop-restoration
filtering/output, reference refresh, AVM/dav2d byte equality, and successful
ac0ej3 decode as unsupported or unclaimed.

#### Scenario: Matrix evidence records the IntrABC record handoff

- **WHEN** decoder support status is validated after the IntrABC
  transform-record handoff
- **THEN** `ac0ej3-selectable-transform-records` remains a partial row with
  Feature ID `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS`
- **AND** the row cites AV2 §5.20.5.3, §5.20.5.4, §5.20.6.1, §5.20.6.3,
  §5.20.7.27, and §8.3.2 as applicable evidence sections
- **AND** it lists focused tests for active `use_intrabc` syntax, retained
  IntrABC defaults, non-IntrABC regression behavior, and the local ac0ej3
  runtime probe
- **AND** it records the next structured unsupported-feature reason reached by
  the local ac0ej3 probe

#### Scenario: Broad IntrABC and reconstruction claims remain absent

- **WHEN** decoder support and feature status are regenerated
- **THEN** broad IntrABC prediction, decoded frame samples, loop-restoration
  filtering/output, reference refresh, AVM/dav2d byte equality, and successful
  ac0ej3 decode remain unclaimed until separately proven
