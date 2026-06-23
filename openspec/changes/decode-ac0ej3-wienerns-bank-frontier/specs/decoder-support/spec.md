## ADDED Requirements

### Requirement: Decoder support tracks ac0ej3 Wiener NS bank parser frontier

The decoder support model SHALL include a partial row for
`DECODE-AC0EJ3-WIENERNS-BANK-FRONTIER` named
`ac0ej3-wienerns-bank-frontier`. The row SHALL describe that the core parser
consumes the ac0ej3-proven intra luma `read_wienerns_filter(0, 0, 0, 1)`
frame-filter-bank syntax while loop-restoration reconstruction, entropy-coded LR
unit syntax, inter reference Wiener state, 10-bit output, and successful ac0ej3
decode remain out of scope.

#### Scenario: support row validates

- **WHEN** `cargo xtask check-decoder-support` validates decoder support metadata
- **THEN** the `ac0ej3-wienerns-bank-frontier` row exists with Feature ID
  `DECODE-AC0EJ3-WIENERNS-BANK-FRONTIER`
- **AND** the row records focused parser/runtime tests and the local ac0ej3
  regression

#### Scenario: generated status remains honest

- **WHEN** decoder support status is generated
- **THEN** it reports only partial parser/runtime support for the ac0ej3 Wiener
  NS bank frontier
- **AND** it does not report loop-restoration reconstruction, 10-bit output, or
  successful ac0ej3 decode as complete
