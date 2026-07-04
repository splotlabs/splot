## ADDED Requirements

### Requirement: local decoder mission IntrABC transform-record mode-info handoff

The decoder SHALL extend `DECODE-SELECTABLE-TRANSFORM-RECORDS` with a
bounded IntrABC mode-info handoff for the local decoder mission Wiener NS LR
selectable-transform path. When AV2 §5.20.5.3 signals `use_intrabc = 1` for a
supported luma/shared block, the runtime SHALL consume the observed §5.20.5.4
`read_intrabc_info()` symbols in spec order, SHALL retain only the IntrABC facts
needed by transform-size and residual syntax contexts, and SHALL remain
fail-closed before IntrABC prediction, decoded samples, reconstruction,
loop-restoration filtering/output, reference refresh, and successful local decoder mission
decode.

#### Scenario: Active IntrABC mode-info is consumed before transform records

- **WHEN** the local decoder mission Wiener NS LR selectable-transform path reaches a
  supported luma/shared block where §5.20.5.3 signals `use_intrabc = 1`
- **THEN** the runtime consumes the observed §5.20.5.4 IntrABC mode-info symbols
  before §5.20.6 transform-size syntax
- **AND** it uses the IntrABC branch defaults for luma/chroma mode facts instead
  of reading ordinary intra luma/chroma mode syntax
- **AND** it advances to the next structured unsupported-feature frontier

#### Scenario: IntrABC syntax drives downstream contexts without output claims

- **WHEN** an admitted IntrABC block reaches selectable transform-size or
  residual syntax
- **THEN** the handoff uses the retained `is_inter = 1`, `fsc_mode = 0`, default
  `YMode = DC_PRED`, and default `UVMode = DC_PRED` facts for the relevant
  syntax contexts
- **AND** it does not populate decoded `CurrFrame` or `CdefFrame` samples
- **AND** it does not claim current-frame block-copy prediction or successful
  local decoder mission decode

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

### Requirement: local decoder mission IntrABC handoff remains no-decode-output

The decoder SHALL NOT treat IntrABC syntax consumption in the local decoder mission
selectable-transform handoff as evidence of decoded output support. The local
probe may advance to a later structured unsupported-feature diagnostic, but
decoded samples, loop-restoration filtering/output, reference refresh,
AVM/dav2d byte equality, and successful local decoder mission decode SHALL remain unclaimed
until separately implemented and proven.

#### Scenario: IntrABC handoff does not complete local decoder mission decode

- **WHEN** the local decoder mission probe advances past the prior
  `unsupported_wienerns_lr_selectable_transform_records_intrabc` diagnostic
- **THEN** the next diagnostic still reports `decode/unsupported-feature`
- **AND** the implementation and support matrices still record local decoder mission decode as
  incomplete
- **AND** no raw, Y4M, or hash output is produced for the full local decoder mission stream
