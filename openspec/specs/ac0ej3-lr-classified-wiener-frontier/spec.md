## Purpose

Track the ac0ej3 fail-closed runtime frontier for AV2 §7.20.4
classified-Wiener dependency derivation.

## Requirements

### Requirement: ac0ej3 Classified Wiener Dependency Frontier

The decoder SHALL track `DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-FRONTIER` as a
fail-closed runtime frontier for ac0ej3 luma Wiener NS LR blocks that require
AV2 §7.20.4 skip-filter pixel classification before §7.20.3 filtering.

#### Scenario: Classified luma dependencies are resolved before rejection

- **WHEN** active luma Wiener NS LR blocks have `frame_filters_on == 1` and
  `NumFilterClasses > 1`
- **THEN** the minimal runtime resolves §7.20.4 classified-luma source-read and
  `LrTxSkip` lookup coordinates before returning `decode/unsupported-feature`
- **AND** the diagnostic SHALL NOT claim source sample values, `LrTxSkip` values,
  `FilterClass` derivation, LR filtering, output, or successful ac0ej3 decode.
