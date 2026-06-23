## ADDED Requirements

### Requirement: Derive directional UV transform type

The decoder SHALL provide a crate-private ordinary coefficient branch handoff,
tracked by `DECODE-COEFF-ORDINARY-BRANCH-DIRECTIONAL-UV-HANDOFF`, that handles
the AV2 section 5.20.7.29 non-lossless intra chroma directional `UVMode` subset
by deriving `pAngle` from `Mode_To_Angle[UVMode]`, caller-resolved
`AngleDeltaUV`, and `ANGLE_STEP`, applying `wide_angle_mapping`, mapping the
resulting mode through `Mode_To_Txfm`, checking intra transform-set membership,
and delegating to the existing transform-size/scan ordinary branch.

#### Scenario: Directional UV mode maps without wide-angle remap

- **WHEN** a nonzero intra chroma ordinary branch uses a directional `UVMode`
  whose block shape does not trigger `wide_angle_mapping`
- **THEN** the handoff SHALL derive the same `PlaneTxType` as
  `Mode_To_Txfm[UVMode]`
- **AND** the resulting ordinary branch behavior SHALL match an explicit
  `PlaneTxType` input

#### Scenario: Directional UV mode maps with wide-angle remap

- **WHEN** a nonzero intra chroma ordinary branch uses a directional `UVMode`,
  caller-resolved `AngleDeltaUV`, and transform dimensions that trigger
  `wide_angle_mapping`
- **THEN** the handoff SHALL map the mode before the `Mode_To_Txfm` lookup
- **AND** the resulting ordinary branch behavior SHALL match an explicit
  `PlaneTxType` input for the remapped mode

#### Scenario: Directional UV fallback honors txSet

- **WHEN** the derived directional `PlaneTxType` is not a member of the resolved
  intra transform set
- **THEN** the handoff SHALL fall back to `DCT_DCT`

#### Scenario: Directional UV remains fail-atomic on invalid domains

- **WHEN** directional mapping receives an invalid `UVMode`, invalid generated
  table value, or invalid transform-size table value
- **THEN** the handoff SHALL return a typed ordinary branch error before
  mutating tile coefficient context state, tile CDF rows, or symbol-decoder
  position

### Requirement: Preserve existing Mode_To_Txfm subset behavior

The directional UV extension SHALL preserve the existing all-zero,
non-directional, chroma-DCT-only, luma/inter/lossless rejection, and
transform-set fallback behavior of the staged `Mode_To_Txfm` handoff.

#### Scenario: Existing non-directional behavior is unchanged

- **WHEN** the handoff receives a non-directional `UVMode`
- **THEN** it SHALL continue to derive `PlaneTxType` from
  `Mode_To_Txfm[UVMode]` without applying `wide_angle_mapping`

#### Scenario: Unsupported subsets remain rejected

- **WHEN** the handoff receives luma, inter, or lossless input
- **THEN** it SHALL keep returning the existing typed unsupported-subset error
  before state, CDF, or symbol mutation
