# validator delta: qm-layer-dependency

Closes the §6.17.6.2 QM layer-dependency residual on `AV2-5.18.6-QUANTIZATION` by joining the
recorded quantizer-matrix level layer identity against the §5.4.1 dependency maps.

## ADDED Requirements

### Requirement: referenced QM levels honor the §6.17.6.2 layer-dependency constraints

The validator SHALL, for an inter/intra frame with `using_qmatrix == 1`, verify for each
referenced custom quantizer-matrix level whose §7.3.8.9-available record has a recorded
`QmMLayerId >= 0` (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md :5413-5419): that the
active sequence header's `MLayerDependencyMap[obu_mlayer_id][QmMLayerId[level]] == 1` and
`TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][QmTLayerId[level]] == 1`. A violation of the
former produces `frame-header/qm-mlayer-dependency-missing`; of the latter,
`frame-header/qm-tlayer-dependency-missing`. A level reset to defaults (`QmMLayerId == -1`) has
no defining layer and is not subject to the constraint. The checks run on the same
availability/poison-guarded, sequence-resolved path as `frame-header/qm-level-unavailable`.

#### Scenario: QM level at an undepended embedded layer

- **WHEN** `using_qmatrix == 1` references a custom level whose defining QM OBU was at an
  embedded layer the frame's `obu_mlayer_id` does not depend on
  (`MLayerDependencyMap[obu_mlayer_id][QmMLayerId] == 0`)
- **THEN** an error diagnostic `frame-header/qm-mlayer-dependency-missing` (§6.17.6.2) is
  produced

#### Scenario: QM level at an undepended temporal layer

- **WHEN** the embedded-layer dependency is satisfied but the frame's `obu_tlayer_id` does not
  depend on the level's defining temporal layer
  (`TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][QmTLayerId] == 0`)
- **THEN** an error diagnostic `frame-header/qm-tlayer-dependency-missing` (§6.17.6.2) is
  produced

#### Scenario: satisfied dependency stays silent

- **WHEN** the frame depends on the level's defining embedded and temporal layers (including
  the reflexive base-layer case)
- **THEN** neither §6.17.6.2 QM layer-dependency diagnostic is produced

#### Scenario: reset-to-defaults level is exempt

- **WHEN** a referenced level was reset to defaults (`QmMLayerId == -1`, no defining layer)
- **THEN** the §6.17.6.2 layer-dependency check is not evaluated for it

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
