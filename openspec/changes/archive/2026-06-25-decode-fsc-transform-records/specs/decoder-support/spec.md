## ADDED Requirements

### Requirement: local decoder mission FSC transform-record support row

The decoder support model SHALL record the local decoder mission FSC transform-record handoff
under `DECODE-SELECTABLE-TRANSFORM-RECORDS`. The support row SHALL
describe that the local decoder mission Wiener NS LR path consumes the observed active
`fsc_mode`/`useFsc` luma residual subcase into retained `LrTxSkip` metadata,
and SHALL continue to mark decoded samples, loop-restoration filtering/output,
reference refresh, AVM/dav2d byte equality, and successful local decoder mission decode as
unsupported or unclaimed.

#### Scenario: Matrix evidence records the FSC record handoff

- **WHEN** decoder support status is validated after the FSC transform-record
  handoff
- **THEN** `selectable-transform-records` remains a partial row with
  Feature ID `DECODE-SELECTABLE-TRANSFORM-RECORDS`
- **AND** the row cites AV2 §5.20.5.3, §5.20.7.27, §5.20.8.2, and §8.3.2 as
  applicable evidence sections
- **AND** it lists focused tests for active `fsc_mode` syntax, selected
  `useFsc` coefficient handoff, non-selected branch preservation, and the local
  local decoder mission runtime probe
- **AND** it records the next structured unsupported-feature reason reached by
  the local decoder mission probe

#### Scenario: Broad FSC and reconstruction claims remain absent

- **WHEN** decoder support and feature status are regenerated
- **THEN** broad FSC/IDTX reconstruction, decoded frame samples,
  loop-restoration filtering/output, reference refresh, AVM/dav2d byte
  equality, and successful local decoder mission decode remain unclaimed until separately
  proven
