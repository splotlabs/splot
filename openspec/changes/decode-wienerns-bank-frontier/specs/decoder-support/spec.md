## ADDED Requirements

### Requirement: Decoder support tracks local decoder mission Wiener NS bank parser frontier

The decoder support model SHALL include a partial row for
`DECODE-WIENERNS-BANK-FRONTIER` named
`wienerns-bank-frontier`. The row SHALL describe that the core parser
consumes the proven intra luma `read_wienerns_filter(0, 0, 0, 1)`
frame-filter-bank syntax while loop-restoration reconstruction, entropy-coded LR
unit syntax, inter reference Wiener state, 10-bit output, and successful local decoder mission
decode remain out of scope.

#### Scenario: support row validates

- **WHEN** `cargo xtask check-decoder-support` validates decoder support metadata
- **THEN** the `wienerns-bank-frontier` row exists with Feature ID
  `DECODE-WIENERNS-BANK-FRONTIER`
- **AND** the row records focused parser/runtime tests and the local decoder mission
  regression

#### Scenario: generated status remains honest

- **WHEN** decoder support status is generated
- **THEN** it reports only partial parser/runtime support for the local decoder mission Wiener
  NS bank frontier
- **AND** it does not report loop-restoration reconstruction, 10-bit output, or
  successful local decoder mission decode as complete
