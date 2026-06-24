## ADDED Requirements

### Requirement: ac0ej3 active intra IST handoff support row

The decoder support model SHALL track
`DECODE-AC0EJ3-ACTIVE-INTRA-IST-HANDOFF` as a distinct partial ac0ej3 row named
`ac0ej3-active-intra-ist-handoff`. The row SHALL describe that the minimal
runtime consumes active AV2 §5.20.7.29 intra IST secondary-transform syntax for
the Wiener NS LR tx-skip record path, records the active-IST metadata, and
remains fail-closed before secondary inverse transforms, decoded samples,
loop-restoration output, reference refresh, or successful ac0ej3 decode.

#### Scenario: Matrix evidence records the active IST handoff boundary

- **WHEN** `cargo xtask check-decoder-support` validates decoder support status
- **THEN** `ac0ej3-active-intra-ist-handoff` appears with Feature ID
  `DECODE-AC0EJ3-ACTIVE-INTRA-IST-HANDOFF`
- **AND** the row cites AV2 §5.20.7.29, §7.20.4, §8.3.2, and the focused tests
  plus the local ac0ej3 runtime probe
- **AND** it does not claim secondary inverse-transform runtime wiring,
  decoded frame samples, loop-restoration output, raw/Y4M output, reference
  refresh, AVM/dav2d byte equality, or successful ac0ej3 decode
