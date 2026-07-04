## ADDED Requirements

### Requirement: local decoder mission LR Classified Wiener Frontier Support Row

The implementation matrix SHALL include
`DECODE-LR-CLASSIFIED-WIENER-FRONTIER` as a distinct local decoder mission support row.
The row SHALL describe the resolved §7.20.4 dependency frontier, the remaining
value/filtering gap, and the live local decoder mission runtime diagnostic.

#### Scenario: Local local decoder mission gate cites classified dependency frontier

- **WHEN** the local decoder mission fixture reaches active luma Wiener NS LR with
  more than one luma filter class
- **THEN** the runtime diagnostic cites
  `lr-classified-wiener-frontier`
- **AND** it uses feature id
  `DECODE-LR-CLASSIFIED-WIENER-FRONTIER`
- **AND** it remains an unsupported-feature diagnostic before output.
