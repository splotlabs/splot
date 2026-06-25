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
and luma-only chroma-offset subcases have been consumed. When the local stream
reaches SDP chroma-part mode-info that depends on §5.20.3.1 `CflAllowedInSdp`,
that prerequisite SHALL be tracked by `DECODE-AC0EJ3-SDP-CFL-ALLOWED-FRONTIER`
until the observed syntax-synchronization subcase has been consumed. When the
local stream reaches luma/shared mode-info prelude syntax (`use_intrabc`, CDEF,
and delta-Q) before the selectable transform-record syntax, that prerequisite
SHALL be tracked by `DECODE-AC0EJ3-INTRA-PRELUDE-TX-FRONTIER` until the
observed prelude and chroma-offset safety subcase has been consumed.

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

### Requirement: ac0ej3 IntrABC transform-record mode-info handoff

The decoder SHALL extend `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS` with a
bounded IntrABC mode-info handoff for the local ac0ej3 Wiener NS LR
selectable-transform path. When AV2 §5.20.5.3 signals `use_intrabc = 1` for a
supported luma/shared block, the runtime SHALL consume the observed §5.20.5.4
`read_intrabc_info()` symbols in spec order, SHALL retain only the IntrABC facts
needed by transform-size and residual syntax contexts, and SHALL remain
fail-closed before IntrABC prediction, decoded samples, reconstruction,
loop-restoration filtering/output, reference refresh, and successful ac0ej3
decode.
For observed NEARMV/NEWMV blocks, the runtime SHALL retain bounded IntrABC
block-vector facts parsed in spec order without claiming current-frame
block-copy prediction or output.

#### Scenario: Active IntrABC mode-info is consumed before transform records

- **WHEN** the local ac0ej3 Wiener NS LR selectable-transform path reaches a
  supported luma/shared block where §5.20.5.3 signals `use_intrabc = 1`
- **THEN** the runtime consumes the observed §5.20.5.4 IntrABC mode-info symbols
  before §5.20.6 transform-size syntax
- **AND** it uses the IntrABC branch defaults for luma/chroma mode facts instead
  of reading ordinary intra luma/chroma mode syntax
- **AND** it advances to the next structured unsupported-feature frontier

#### Scenario: active IntrABC NEWMV block-vector syntax is handed off

- **WHEN** the local ac0ej3 selectable-transform path reaches a supported
  luma/shared block with `use_intrabc = 1`
- **AND** §5.20.5.4 decodes `intrabc_mode = 0`
- **THEN** the runtime SHALL read the optional `intrabc_precision`, derive the
  bounded IntrABC reference block-vector candidates, consume §5.20.7.20
  `read_mv()` using `MV_INTRABC_CONTEXT` and the decoded `MvPrecision`, apply
  §5.20.7.13 `mv_clamp_to_integer`, and retain the resulting block vector.
- **AND** it SHALL stop before current-frame block-copy prediction with a
  structured unsupported diagnostic.

#### Scenario: active IntrABC NEARMV block-vector syntax is handed off

- **WHEN** the local ac0ej3 selectable-transform path reaches a supported
  luma/shared block with `use_intrabc = 1`
- **AND** §5.20.5.4 decodes `intrabc_mode = 1`
- **THEN** the runtime SHALL select the retained DRL candidate from the bounded
  IntrABC reference block-vector stack without reading `read_mv()`.
- **AND** it SHALL keep the handoff syntax-only, without claiming current-frame
  block-copy prediction or output.

#### Scenario: IntrABC syntax drives downstream contexts without output claims

- **WHEN** an admitted IntrABC block reaches selectable transform-size or
  residual syntax
- **THEN** the handoff uses the retained `is_inter = 1`, `fsc_mode = 0`, default
  `YMode = DC_PRED`, and default `UVMode = DC_PRED` facts for the relevant
  syntax contexts
- **AND** it does not populate decoded `CurrFrame` or `CdefFrame` samples
- **AND** it does not claim current-frame block-copy prediction or successful
  ac0ej3 decode

#### Scenario: Unsupported IntrABC branches stay fail-closed

- **WHEN** the IntrABC mode-info path reaches an unobserved or unsupported
  sub-branch
- **THEN** the runtime returns a structured `decode/unsupported-feature`
  diagnostic
- **AND** it does not populate partial or fabricated `LrTxSkip` values

#### Scenario: Non-IntrABC prelude behavior is preserved

- **WHEN** §5.20.5.3 either does not code `use_intrabc` or decodes
  `use_intrabc = 0`
- **THEN** the selectable-transform path preserves the existing ordinary intra
  prelude, luma/chroma mode, transform partition, and residual handoff behavior

### Requirement: ac0ej3 IntrABC handoff remains no-decode-output

The decoder SHALL NOT treat IntrABC syntax consumption in the ac0ej3
selectable-transform handoff as evidence of decoded output support. The local
probe may advance to a later structured unsupported-feature diagnostic, but
decoded samples, loop-restoration filtering/output, reference refresh,
AVM/dav2d byte equality, and successful ac0ej3 decode SHALL remain unclaimed
until separately implemented and proven.

#### Scenario: IntrABC handoff does not complete ac0ej3 decode

- **WHEN** the local ac0ej3 probe advances past the prior
  `unsupported_wienerns_lr_selectable_transform_records_intrabc` diagnostic
- **THEN** the next diagnostic still reports `decode/unsupported-feature`
- **AND** the implementation and support matrices still record ac0ej3 decode as
  incomplete
- **AND** no raw, Y4M, or hash output is produced for the full ac0ej3 stream

### Requirement: ac0ej3 luma-only narrow selectable record handoff

The decoder SHALL track `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS` as the
owner of the bounded luma-only narrow selectable-record handoff. When the local
ac0ej3 Wiener NS LR path reaches a supported luma-only narrow selectable leaf,
the runtime SHALL consume AV2 §5.20.6.3 partition syntax and, when applying the
consumed partition would produce empty geometry, SHALL retain the leaf's actual
4x4-grid extent as a `SelectableLumaTxRecord`. It SHALL avoid fabricating
broader max-rectangle transform cells outside that leaf and SHALL remain
fail-closed before decoded sample population or loop-restoration output.

#### Scenario: Narrow luma leaf falls back after empty partition geometry

- **WHEN** the local ac0ej3 mission stream reaches a luma-only narrow selectable
  transform leaf
- **AND** the runtime consumes the required transform-partition syntax
- **AND** applying the consumed partition would derive a zero-width or
  zero-height subrecord for that leaf
- **THEN** the runtime records the actual luma leaf extent instead
- **AND** it advances past
  `unsupported_wienerns_lr_selectable_transform_records_empty_transform`
- **AND** it stops at the next structured unsupported-feature frontier

#### Scenario: Luma-only chroma-offset narrow leaf is retained

- **WHEN** the selectable transform-record path reaches a chroma-offset narrow
  leaf that is still in the luma partition and has no chroma residual syntax
- **THEN** the runtime records the actual luma leaf extent
- **AND** it does not invoke chroma residual coordinate handoff
- **AND** chroma-bearing offset leaves still fail closed with a structured
  unsupported-feature diagnostic

#### Scenario: General zero geometry remains invalid

- **WHEN** selectable transform partitioning outside the supported luma-only
  narrow fallback derives a zero-width or zero-height transform record
- **THEN** the runtime rejects the path with a structured unsupported-feature
  diagnostic
- **AND** it does not populate fabricated transform cells or partial `LrTxSkip`
  values

#### Scenario: Skipped luma residuals are retained

- **WHEN** an admitted selectable luma transform record consumes AV2 §5.20.7.27
  `all_zero` syntax with `all_zero == 1`
- **THEN** the LR tx-skip handoff records `skip_flag = true` and `eob = 0` for
  that transform extent
- **AND** the retained record remains available for live `LrTxSkip` population

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
