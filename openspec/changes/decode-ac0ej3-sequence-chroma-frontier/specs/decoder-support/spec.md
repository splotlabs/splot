## ADDED Requirements

### Requirement: Decoder support tracks ac0ej3 sequence chroma frontier

The decoder support model SHALL include a partial row for
`DECODE-AC0EJ3-SEQUENCE-CHROMA-FRONTIER` named
`ac0ej3-sequence-chroma-frontier`. The row SHALL describe that the runtime parses
sequence-level CfL/MHCCP capability flags but still rejects them at the pre-tile
runtime boundary before any unimplemented §5.20.5.6 chroma mode-info syntax can be
skipped.

#### Scenario: support row validates

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  metadata
- **THEN** the `ac0ej3-sequence-chroma-frontier` row exists with Feature ID
  `DECODE-AC0EJ3-SEQUENCE-CHROMA-FRONTIER`
- **AND** the row records the focused runtime and local `ac0ej3` regression tests

#### Scenario: status does not overclaim

- **WHEN** decoder support status is generated
- **THEN** CfL prediction, MHCCP prediction, `read_cfl_alphas`, 10-bit
  reconstruction/output, key-frame loop filters, and full `ac0ej3` decode remain
  partial or unsupported until separately implemented and proven
