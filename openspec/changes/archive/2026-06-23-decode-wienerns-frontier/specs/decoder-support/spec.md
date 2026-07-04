## ADDED Requirements

### Requirement: Decoder support tracks local decoder mission Wiener NS frontier

The decoder support model SHALL include a partial row for
`DECODE-WIENERNS-FRONTIER` named `wienerns-frontier`. The row
SHALL describe that the runtime surfaces the current local decoder mission key-frame header
coverage stop at AV2 5.18.7.11, where `lr_params()` reaches
`read_wienerns_filter()`, while keeping Wiener NS syntax parsing,
loop-restoration filtering, 10-bit reconstruction/output, and full local decoder mission decode
out of scope.

#### Scenario: support row validates

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  metadata
- **THEN** the `wienerns-frontier` row exists with Feature ID
  `DECODE-WIENERNS-FRONTIER`
- **AND** the row records focused runtime and local decoder mission regression tests

#### Scenario: generated status remains honest

- **WHEN** decoder support status is generated
- **THEN** Wiener NS filter-bank decode, loop-restoration reconstruction, 10-bit
  reconstruction/output, and successful local decoder mission decode remain partial or
  unsupported until separately implemented and proven
