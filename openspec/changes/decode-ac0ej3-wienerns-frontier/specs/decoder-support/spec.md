## ADDED Requirements

### Requirement: Decoder support tracks ac0ej3 Wiener NS frontier

The decoder support model SHALL include a partial row for
`DECODE-AC0EJ3-WIENERNS-FRONTIER` named `ac0ej3-wienerns-frontier`. The row
SHALL describe that the runtime surfaces the current ac0ej3 key-frame header
coverage stop at AV2 5.18.7.11, where `lr_params()` reaches
`read_wienerns_filter()`, while keeping Wiener NS syntax parsing,
loop-restoration filtering, 10-bit reconstruction/output, and full ac0ej3 decode
out of scope.

#### Scenario: support row validates

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  metadata
- **THEN** the `ac0ej3-wienerns-frontier` row exists with Feature ID
  `DECODE-AC0EJ3-WIENERNS-FRONTIER`
- **AND** the row records focused runtime and local ac0ej3 regression tests

#### Scenario: generated status remains honest

- **WHEN** decoder support status is generated
- **THEN** Wiener NS filter-bank decode, loop-restoration reconstruction, 10-bit
  reconstruction/output, and successful ac0ej3 decode remain partial or
  unsupported until separately implemented and proven
