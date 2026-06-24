# ac0ej3 Selectable Transform Records Specification

## Purpose
Define the fail-closed ac0ej3 Wiener NS loop-restoration frontier that parses
supported `TX_MODE_SELECT` luma transform records and hands their `LrTxSkip`
facts into live storage before decoded sample population is supported.

## Requirements

### Requirement: ac0ej3 Selectable Transform Records

The decoder SHALL track `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS` as a
partial runtime prerequisite for the ac0ej3 Wiener NS LR path. For supported
`TX_MODE_SELECT` intra luma blocks, the runtime SHALL parse AV2 §5.20.6.1
`read_tx_size` and §5.20.6.3 `read_tx_partition` transform-size syntax, SHALL
use the resulting luma transform extents when reading §5.20.7.27 coefficients,
and SHALL hand those luma transform records into live `LrTxSkip` storage without
fabricating missing values. The runtime SHALL also keep the syntax-only LR
handoff aligned with AV2 §5.20.7.24, §5.20.7.25, §5.20.7.27, and §5.20.7.30
while consuming the transform-record residual subcase required by the local
ac0ej3 stream. When the local stream reaches active CfL chroma mode syntax while
deriving those records, the active CfL prerequisite SHALL be tracked by
`DECODE-AC0EJ3-CFL-CHROMA-MODE-FRONTIER` and SHALL not be counted as completed
selectable-transform support until its syntax has been consumed. When the local
stream reaches luma-only narrow SDP transform records, that prerequisite SHALL
be tracked by
`DECODE-AC0EJ3-SELECTABLE-NARROW-LUMA-RECORDS` until the observed luma-only
subcase has been consumed. When the local stream reaches SDP chroma-part
mode-info that depends on §5.20.3.1 `CflAllowedInSdp`, that prerequisite SHALL
be tracked by `DECODE-AC0EJ3-SDP-CFL-ALLOWED-FRONTIER` until the observed
syntax-synchronization subcase has been consumed. When the local stream reaches
luma/shared mode-info prelude syntax (`use_intrabc`, CDEF, and delta-Q) before
the selectable transform-record syntax, that prerequisite SHALL be tracked by
`DECODE-AC0EJ3-INTRA-PRELUDE-TX-FRONTIER` until the observed prelude and
chroma-offset safety subcase has been consumed.

#### Scenario: Selectable records populate live tx-skip storage

- **WHEN** the local ac0ej3 mission stream reaches active luma Wiener NS LR
- **AND** its key frame uses supported `TX_MODE_SELECT` transform records
- **AND** the required chroma mode-info prerequisites, including active CfL when
  present, have been consumed
- **AND** the required luma-only narrow transform-record prerequisites have been
  consumed
- **AND** the required SDP `CflAllowedInSdp` chroma mode-info prerequisites have
  been consumed
- **AND** the required intra prelude transform-record prerequisites have been
  consumed
- **THEN** the runtime derives a complete `WienerNsLrTxSkipGrid`
- **AND** it populates the live LR `LrTxSkip` shell with tile-derived values
- **AND** it advances past the
  `unsupported_wienerns_lr_tx_mode_select_transform_records` diagnostic

#### Scenario: Transform-record residual syntax remains geometry-checked

- **WHEN** the local ac0ej3 mission stream reaches active transform-record
  residual syntax after live `LrTxSkip` values are available
- **THEN** the runtime consumes the supported residual subcase with AV2-derived
  transform sizes and scan lengths
- **AND** invalid EOB/scan combinations still fail closed as residual parse
  errors

#### Scenario: Unsupported selectable transform syntax remains fail-closed

- **WHEN** a selectable transform branch is outside the implemented subset
- **THEN** the runtime returns a structured `decode/unsupported-feature`
  diagnostic
- **AND** it does not populate partial or fabricated `LrTxSkip` values

#### Scenario: No successful ac0ej3 decode claim

- **WHEN** selectable transform records have populated live `LrTxSkip`
- **THEN** the decoder SHALL NOT claim decoded `CurrFrame` or `CdefFrame`
  samples, `FilterClass`, `SubclassLookup`, loop-restoration filtering/output,
  reference refresh, AVM/dav2d byte equality, or successful ac0ej3 decode
