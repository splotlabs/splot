## ADDED Requirements

### Requirement: ac0ej3 LR Classified Wiener Frontier Support Row

The implementation matrix SHALL include
`DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-FRONTIER` as a distinct ac0ej3 support row.
The row SHALL describe the resolved §7.20.4 dependency frontier, the remaining
value/filtering gap, and the live ac0ej3 runtime diagnostic.

#### Scenario: Local ac0ej3 gate cites classified dependency frontier

- **WHEN** the local ac0ej3 mission fixture reaches active luma Wiener NS LR with
  more than one luma filter class
- **THEN** the runtime diagnostic cites
  `ac0ej3-lr-classified-wiener-frontier`
- **AND** it uses feature id
  `DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-FRONTIER`
- **AND** it remains an unsupported-feature diagnostic before output.
