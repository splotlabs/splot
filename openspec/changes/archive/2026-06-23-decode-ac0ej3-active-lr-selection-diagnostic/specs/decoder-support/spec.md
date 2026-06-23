## ADDED Requirements

### Requirement: Active LR Unit Diagnostic Uses Selection Frontier

The decoder support model SHALL report the live
`unsupported_active_wienerns_lr_units` runtime diagnostic under
`DECODE-AC0EJ3-LR-UNIT-SELECTIONS-FRONTIER` once the runtime has retained
per-unit Wiener NS LR selection state. The diagnostic SHALL keep active
`RESTORE_WIENER_NONSEP` units fail-closed before decoded-frame allocation,
reference retention, hash, raw, Y4M, or any successful output path.

#### Scenario: Local ac0ej3 gate cites selection state

- **WHEN** the local ac0ej3 mission fixture reaches the active Wiener NS LR-unit
  runtime gate
- **THEN** the JSON diagnostic keeps reason
  `unsupported_active_wienerns_lr_units`
- **AND** it cites matrix row `ac0ej3-lr-unit-selections-frontier`
- **AND** it cites Feature ID `DECODE-AC0EJ3-LR-UNIT-SELECTIONS-FRONTIER`
- **AND** it does not claim loop-restoration reconstruction, 10-bit output,
  reference refresh, raw/Y4M output, or successful ac0ej3 decode
