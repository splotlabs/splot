## ADDED Requirements

### Requirement: ac0ej3 luma-only narrow selectable record handoff

The decoder SHALL track `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS` as the
owner of the bounded luma-only narrow selectable-record handoff. When the local
ac0ej3 Wiener NS LR path reaches a supported luma-only narrow selectable leaf,
the runtime SHALL retain the leaf's actual 4x4-grid extent as a
`SelectableLumaTxRecord`, SHALL avoid fabricating broader max-rectangle
transform cells outside that leaf, and SHALL remain fail-closed before decoded
sample population or loop-restoration output.

#### Scenario: Narrow luma leaf bypasses impossible empty transform partition

- **WHEN** the local ac0ej3 mission stream reaches a luma-only narrow selectable
  transform leaf
- **AND** max-rectangle transform partitioning would derive a zero-width or
  zero-height subrecord for that leaf
- **THEN** the runtime records the actual luma leaf extent instead
- **AND** it advances past
  `unsupported_wienerns_lr_selectable_transform_records_empty_transform`
- **AND** it stops at the next structured unsupported-feature frontier

#### Scenario: Luma-only chroma-offset narrow leaf is retained

- **WHEN** the selectable transform-record path reaches a chroma-offset narrow
  leaf that is still in the luma partition and has no chroma residual syntax
- **THEN** the runtime records the actual luma leaf extent
- **AND** it does not invoke chroma residual coordinate handoff
- **AND** chroma-bearing offset leaves still fail closed with a structured
  unsupported-feature diagnostic

#### Scenario: General zero geometry remains invalid

- **WHEN** selectable transform partitioning outside the supported luma-only
  narrow bypass derives a zero-width or zero-height transform record
- **THEN** the runtime rejects the path with a structured unsupported-feature
  diagnostic
- **AND** it does not populate fabricated transform cells or partial `LrTxSkip`
  values

#### Scenario: Skipped luma residuals are retained

- **WHEN** an admitted selectable luma transform record consumes AV2 §5.20.7.27
  `all_zero` syntax with `all_zero == 1`
- **THEN** the LR tx-skip handoff records `skip_flag = true` and `eob = 0` for
  that transform extent
- **AND** the retained record remains available for live `LrTxSkip` population
