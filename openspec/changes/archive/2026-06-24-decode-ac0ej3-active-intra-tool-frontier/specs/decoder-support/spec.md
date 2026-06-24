## ADDED Requirements

### Requirement: ac0ej3 active intra tool support row

The decoder support model SHALL track `DECODE-AC0EJ3-ACTIVE-INTRA-TOOL-FRONTIER` as a distinct partial ac0ej3 row named `ac0ej3-active-intra-tool-frontier`. The row SHALL describe that selectable Wiener NS LR transform-record derivation consumes inactive MRL syntax and relaxes broad sequence-level intra/transform tool gates plus parsed CCSO filter state into active-use or later filter/output diagnostics, while remaining fail-closed before decoded frame samples, loop-restoration filtering/output, reference refresh, AVM/dav2d byte equality, or successful ac0ej3 decode.

#### Scenario: Matrix evidence records active intra tool boundary

- **WHEN** decoder support status is validated
- **THEN** `ac0ej3-active-intra-tool-frontier` appears with Feature ID `DECODE-AC0EJ3-ACTIVE-INTRA-TOOL-FRONTIER`
- **AND** the row cites AV2 §5.20.5.5, §5.20.7.27, and §9.3
- **AND** it lists focused MRL CDF/mode-info tests plus the local ac0ej3 runtime probe
- **AND** it does not claim decoded frame samples, loop-restoration filtering, output, reference refresh, AVM/dav2d byte equality, or successful ac0ej3 decode
