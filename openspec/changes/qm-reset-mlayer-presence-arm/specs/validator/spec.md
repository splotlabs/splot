# validator delta: qm-reset-mlayer-presence-arm

Closes the § 5.18.2 `reset_qm()` SWITCH/RAS `MLayerPresenceMap` residual on the quantizer-matrix
availability model by deriving the § 5.4.1 `MLayerPresenceMap` and wiring its arm.

## ADDED Requirements

### Requirement: the reset_qm() SWITCH/RAS MLayerPresenceMap arm clears presence-required levels

The validator SHALL, when applying a CONFIRMED § 5.18.2 `reset_qm()` for an `OBU_SWITCH`
(with `restricted_prediction_switch == 1`) or `OBU_RAS_FRAME`
(docs/spec/av2/1.0.0/05-syntax-structures.md :5350-5352), clear an unprotected
quantizer-matrix level whose recorded `QmMLayerId == m` when
`MLayerPresenceMap[m][obu_mlayer_id] == 1`, in addition to the existing `QmMLayerId == -1`
arm. `MLayerPresenceMap` is the § 5.4.1 reflexive-transitive closure of the activated sequence
header's `MLayerDependencyMap` (:583-601). When the activated sequence header (and thus the
presence map) is unavailable, the level is left available (the zero-false-positive direction
for an availability reset). A cleared level subsequently referenced without a resend fires the
existing `frame-header/qm-level-unavailable` (§ 7.3.8.9).

#### Scenario: a same-layer random-access reset clears the level

- **WHEN** a quantizer-matrix OBU at embedded layer `m` makes a custom level available, then a
  confirmed SWITCH/RAS `reset_qm()` runs at `obu_mlayer_id == m` (so
  `MLayerPresenceMap[m][m] == 1`, reflexive), and a later frame references the level without a
  resend
- **THEN** `frame-header/qm-level-unavailable` (§ 7.3.8.9) is produced

#### Scenario: a cross-layer reset preserves the level

- **WHEN** the level's `QmMLayerId == m` and the confirmed SWITCH/RAS `reset_qm()` runs at an
  `obu_mlayer_id` with `MLayerPresenceMap[m][obu_mlayer_id] == 0` (the current layer is not
  present when `m` is decoded)
- **THEN** the level remains available and no `frame-header/qm-level-unavailable` is produced
  for it

#### Scenario: an unresolved activation does not reset

- **WHEN** the frame's activated sequence header (and thus its `MLayerPresenceMap`) cannot be
  resolved
- **THEN** the presence arm clears no level (the level is left available)

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
