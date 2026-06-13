# Change: qm-reset-mlayer-presence-arm

## Feature IDs

- `AV2-5.4.1-SEQUENCE-HEADER-GENERAL`
- `AV2-5.18.6-QUANTIZATION`
- `AV2-5.13-QUANTIZATION-MATRIX`

## Why

The § 5.18.2 `reset_qm()` SWITCH/RAS arm
(docs/spec/av2/1.0.0/05-syntax-structures.md :5350-5352) is:

```text
needsReset = QmMLayerId[level] == -1 || MLayerPresenceMap[QmMLayerId[level]][obu_mlayer_id]
```

The validator modeled only the `QmMLayerId == -1` clause, leaving the
`MLayerPresenceMap[...]` clause a named residual — so a SWITCH/RAS frame did NOT reset a
quantizer-matrix level whose recorded `QmMLayerId == m` even when the spec requires it. The
dominant case is single-layer: `QmMLayerId == obu_mlayer_id == 0`, so
`MLayerPresenceMap[0][0] == 1` (reflexive) and a same-layer random-access/switch frame resets
the level. Leaving this unmodeled UNDER-reported `frame-header/qm-level-unavailable`
(§ 7.3.8.9): a level reset at a RAS but referenced afterward without a resend was wrongly
treated as available.

`MLayerPresenceMap` is the § 5.4.1 reflexive-transitive closure of the already-modeled
`MLayerDependencyMap` — a pure derivation requiring no new parsing. Wiring it closes the
residual and tightens the §7.3.8.9 check, with zero false positives (the presence map is
static sequence-header state; the current frame's `obu_mlayer_id` is the OBU header; an
unresolved activation leaves the level available — the safe direction for an availability
reset).

## Scope

- Spec sections: § 5.4.1 (MLayerPresenceMap derivation), § 5.18.2 (`reset_qm()` SWITCH/RAS
  arm).
- `crates/splot-core/src/headers/sequence.rs`: derive `MLayerPresenceMap` from
  `MLayerDependencyMap` (`presence_map()` + `is_present()`); a transitive-closure unit test.
- `crates/splot-validate/src/context/quantizer_matrix.rs`: `reset_qm_availability_for_switch_or_ras`
  takes `obu_mlayer_id` + the activated header's `MLayerPresenceMap` and adds the presence
  arm; `apply_qm_reset_for_frame` derives the presence map from the resolved sequence header.
- No new diagnostic (feeds the existing `frame-header/qm-level-unavailable`); no
  `VALIDATOR-DIAGNOSTICS.md` change.

## Non-goals

- The § 6.17.6.2 `QmMLayerId`/`QmTLayerId` layer-dependency constraints on the QM *reference*
  (a separate TODO) — still deferred.
- The QM RAP-replay (`AV2-7.3.8-HLS-AVAILABILITY`) already covers the §7.3.8.1 direction.

## Acceptance criteria

- [ ] `MLayerPresenceMap` derived as the reflexive-transitive closure of `MLayerDependencyMap`,
      with a test proving a transitive edge a direct dependency lacks.
- [ ] The `reset_qm()` SWITCH/RAS presence arm resets a same-layer level
      (`MLayerPresenceMap[m][obu_mlayer_id] == 1`) and preserves a cross-layer level
      (`== 0`).
- [ ] The QM RAP-replay disjointness test is reconstructed multi-layer (a layer-0 level
      survives a layer-1 RAS, so the replay fires while the linear check stays silent).
- [ ] An unresolved activation leaves levels available (no false unavailability).
- [ ] `cargo xtask ci` passes.
