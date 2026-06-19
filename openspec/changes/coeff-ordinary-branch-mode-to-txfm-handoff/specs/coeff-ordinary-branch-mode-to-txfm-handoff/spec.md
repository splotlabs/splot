## ADDED Requirements

### Requirement: Derive non-directional intra chroma PlaneTxType from Mode_To_Txfm

The decoder SHALL provide a crate-private ordinary coefficient branch handoff
that derives `PlaneTxType` for the non-lossless intra chroma non-directional
subset of AV2 §5.20.7.29 from generated
`splot-core::tables::conversion::MODE_TO_TXFM` and the inline
`Tx_Type_In_Set_Intra` membership table. The handoff SHALL honor the
caller-resolved `enable_chroma_dctonly` short-circuit, SHALL delegate
successful nonzero branches to the existing caller-resolved `PlaneTxType`
ordinary branch, and SHALL preserve the all-zero branch behavior.

#### Scenario: Mapped transform is accepted

- **WHEN** the handoff receives a non-lossless intra chroma nonzero branch with
  a valid non-directional `UVMode` whose mapped transform is allowed by the
  caller-resolved `txSet`
- **THEN** it derives the mapped `PlaneTxType` from `MODE_TO_TXFM`
- **AND** the resulting ordinary branch behavior matches the existing explicit
  `PlaneTxType` handoff

#### Scenario: Mapped transform falls back to DCT_DCT

- **WHEN** the handoff receives a valid non-directional `UVMode` whose mapped
  transform is not allowed by the caller-resolved intra transform set
- **THEN** it derives `DCT_DCT` before delegating to the existing ordinary branch

#### Scenario: Chroma-dct-only short-circuit falls back to DCT_DCT

- **WHEN** the handoff receives a valid non-lossless intra chroma nonzero branch
  with caller-resolved `enable_chroma_dctonly` set
- **THEN** it derives `DCT_DCT` before the `Mode_To_Txfm` lookup and before
  delegating to the existing ordinary branch

#### Scenario: Unsupported subset fails atomically

- **WHEN** the handoff receives luma, inter, lossless, directional, invalid
  `UVMode`, or invalid `txSet` inputs for the nonzero branch
- **THEN** it returns a typed ordinary-branch error before mutating coefficient
  context state, tile CDF rows, or symbol-decoder state

#### Scenario: Runtime scope remains unchanged

- **WHEN** the minimal runtime and existing staged ordinary-branch paths run
- **THEN** they remain no-output-change
- **AND** full `compute_tx_type`, `get_tx_set`, directional wide-angle mapping,
  luma/inter/lossless branches, runtime `coeffs()`, dequantization,
  reconstruction, output, and reference refresh remain unsupported
