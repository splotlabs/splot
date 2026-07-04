## ADDED Requirements

### Requirement: local decoder mission Inactive LR Unit Frontier Support Row
The decoder support model SHALL track
`DECODE-INACTIVE-LR-UNITS-FRONTIER` as a distinct local decoder mission support row.
The row SHALL describe that the minimal runtime consumes the supported
frame-level Wiener NS LR-unit syntax, distinguishes `RESTORE_NONE` units from
active `RESTORE_WIENER_NONSEP` units, and only advances beyond the LR frontier
when every consumed unit is inactive. The row SHALL NOT claim active
loop-restoration filtering, 10-bit reconstruction/output, reference refresh,
raw/Y4M output, or successful local decoder mission decode.

#### Scenario: Inactive LR units advance to the next true frontier
- **WHEN** the local decoder mission key frame consumes supported frame-level Wiener NS
  LR units and all consumed units select `RESTORE_NONE`
- **THEN** the runtime does not emit `unsupported_wienerns_lr_unit_syntax`
- **AND** it either reaches the next structured unsupported diagnostic or a
  later supported runtime path without claiming loop-restoration filtering

#### Scenario: Active LR units remain unsupported
- **WHEN** a minimal-runtime input consumes a supported frame-level Wiener NS LR
  unit that selects `RESTORE_WIENER_NONSEP`
- **THEN** the runtime emits a structured `decode/unsupported-feature`
  diagnostic before decoded-frame allocation, reference retention, hash, raw, or
  Y4M output
- **AND** the diagnostic identifies active Wiener NS loop-restoration
  reconstruction as unsupported

#### Scenario: Matrix evidence records the narrow boundary
- **WHEN** decoder support status is validated
- **THEN** `inactive-lr-units-frontier` remains partial
- **AND** the row lists tests proving inactive LR-unit advancement, active
  LR-unit rejection, resource-limit preservation, and local decoder mission diagnostic
  identity
