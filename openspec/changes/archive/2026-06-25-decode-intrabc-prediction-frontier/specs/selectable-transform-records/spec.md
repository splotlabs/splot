## MODIFIED Requirements

### Requirement: local decoder mission IntrABC transform-record mode-info handoff

The decoder SHALL extend `DECODE-SELECTABLE-TRANSFORM-RECORDS` with a
bounded IntrABC mode-info and prediction-geometry handoff for the local decoder mission
Wiener NS LR selectable-transform path. When AV2 §5.20.5.3 signals
`use_intrabc = 1` for a supported luma/shared block, the runtime SHALL consume
the observed §5.20.5.4 `read_intrabc_info()` symbols in spec order, SHALL retain
only the IntrABC facts needed by transform-size and residual syntax contexts,
SHALL derive bounded luma current-frame prediction source/target geometry from
the retained §5.20.7.13/§5.20.7.20 block vector using the BILINEAR footprint,
current tile bounds, padded MI-domain bounds, and conservative §6.19.7.12
source-overlap rejection, and SHALL remain fail-closed before fabricated decoded
samples, chroma IntrABC prediction, residual reconstruction, loop-restoration
filtering/output, reference refresh, and successful local decoder mission decode.
For observed NEARMV/NEWMV blocks, the runtime SHALL retain bounded IntrABC
block-vector facts parsed in spec order, SHALL hand off only checked luma
current-frame prediction geometry whose source sample envelope stays inside the
current tile and padded MI-domain current-frame storage without overlapping the
current target block, and SHALL report a structured missing-populated-`CurrFrame`
frontier while the live local decoder mission path lacks decoded current-frame samples.

#### Scenario: Active IntrABC mode-info is consumed before transform records

- **WHEN** the local decoder mission Wiener NS LR selectable-transform path reaches a
  supported luma/shared block where §5.20.5.3 signals `use_intrabc = 1`
- **THEN** the runtime consumes the observed §5.20.5.4 IntrABC mode-info symbols
  before §5.20.6 transform-size syntax
- **AND** it uses the IntrABC branch defaults for luma/chroma mode facts instead
  of reading ordinary intra luma/chroma mode syntax
- **AND** it advances to the next structured unsupported-feature frontier

#### Scenario: active IntrABC NEWMV block-vector syntax is handed off

- **WHEN** the local decoder mission selectable-transform path reaches a supported
  luma/shared block with `use_intrabc = 1`
- **AND** §5.20.5.4 decodes `intrabc_mode = 0`
- **THEN** the runtime SHALL read the optional `intrabc_precision`, derive the
  bounded IntrABC reference block-vector candidates, consume §5.20.7.20
  `read_mv()` using `MV_INTRABC_CONTEXT` and the decoded `MvPrecision`, apply
  §5.20.7.13 `mv_clamp_to_integer`, and retain the resulting block vector.
- **AND** it SHALL derive checked tile-local luma current-frame prediction
  target/source geometry and subpel phase before reporting the next structured
  unsupported frontier.
- **AND** it SHALL reject source envelopes that cross the current tile bounds
  or overlap the current target block before decoded samples are available.

#### Scenario: active IntrABC NEARMV block-vector syntax is handed off

- **WHEN** the local decoder mission selectable-transform path reaches a supported
  luma/shared block with `use_intrabc = 1`
- **AND** §5.20.5.4 decodes `intrabc_mode = 1`
- **THEN** the runtime SHALL select the retained DRL candidate from the bounded
  IntrABC fallback block-vector stack without reading `read_mv()` only when the
  tile-local prelude state proves the §7.12.2 stack has no prior IntrABC
  spatial/ref-MV-bank candidates.
- **AND** it SHALL derive checked luma current-frame prediction geometry when
  the retained candidate is in the supported subset
- **AND** it SHALL keep decoded sample population and output unclaimed while the
  live path lacks a populated `CurrFrame`.

#### Scenario: IntrABC syntax drives downstream contexts without output claims

- **WHEN** an admitted IntrABC block reaches selectable transform-size or
  residual syntax
- **THEN** the handoff uses the retained `is_inter = 1`, `fsc_mode = 0`, default
  `YMode = DC_PRED`, and default `UVMode = DC_PRED` facts for the relevant
  syntax contexts
- **AND** it does not fabricate decoded `CurrFrame` or `CdefFrame` samples
- **AND** it does not claim successful current-frame block-copy output or
  successful local decoder mission decode

#### Scenario: Unsupported IntrABC branches stay fail-closed

- **WHEN** the IntrABC mode-info or prediction-geometry path reaches an
  unobserved or unsupported sub-branch
- **THEN** the runtime returns a structured `decode/unsupported-feature`
  diagnostic
- **AND** it does not populate partial or fabricated `LrTxSkip`,
  `CurrFrame`, or `CdefFrame` values

#### Scenario: Non-IntrABC prelude behavior is preserved

- **WHEN** §5.20.5.3 either does not code `use_intrabc` or decodes
  `use_intrabc = 0`
- **THEN** the selectable-transform path preserves the existing ordinary intra
  prelude, luma/chroma mode, transform partition, and residual handoff behavior

### Requirement: local decoder mission IntrABC handoff remains no-decode-output

The decoder SHALL NOT treat IntrABC prediction-geometry handoff in the local decoder mission
selectable-transform path as evidence of decoded output support. The local probe
may advance to a later structured unsupported-feature diagnostic, but decoded
samples, loop-restoration filtering/output, reference refresh, AVM/dav2d byte
equality, and successful local decoder mission decode SHALL remain unclaimed until separately
implemented and proven.

#### Scenario: IntrABC handoff does not complete local decoder mission decode

- **WHEN** the local decoder mission probe advances past the prior
  `unsupported_wienerns_lr_selectable_transform_records_intrabc_prediction`
  diagnostic
- **THEN** the next diagnostic still reports `decode/unsupported-feature`
- **AND** the implementation and support matrices still record local decoder mission decode as
  incomplete
- **AND** no raw, Y4M, or hash output is produced for the full local decoder mission stream
