# Change: qm-layer-dependency

## Feature IDs

- `AV2-5.18.6-QUANTIZATION`

## Why

Closes the §6.17.6.2 QM layer-dependency residual — the long-standing
`TODO(spec: AV2-5.18.6-QUANTIZATION)` in `frame_qm_reference_checks`. §6.17.6.2
(docs/spec/av2/1.0.0/06-syntax-structures-semantics.md :5413-5419, parallel for qm_u/qm_v
:5428-5447) requires, for each referenced custom quantizer-matrix level whose defining QM OBU
recorded a layer identity (`QmMLayerId[level] >= 0`):

```text
MLayerDependencyMap[obu_mlayer_id][QmMLayerId[level]] == 1
TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][QmTLayerId[level]] == 1
```

i.e. the frame's embedded/temporal layer must depend on the level's defining layer. Every
operand is already modeled: the recorded `QmLevelRecord.{mlayer_id, tlayer_id}` (QmMLayerId /
QmTLayerId), the frame's `obu_mlayer_id` / `obu_tlayer_id` (OBU header), and the activated
sequence header's §5.4.1 `MLayerDependencyMap` / `TLayerDependencyMap`. The check directly
mirrors the already-merged `frame-header/film-grain-{mlayer,tlayer}-dependency-missing`
(§6.17.10.1) — same data sources, same dependency-map API, same availability/poison guard.

## Scope

- Spec section: §6.17.6.2 (mirror :5413-5419).
- `crates/splot-validate/src/context/quantizer_matrix.rs`: in `frame_qm_reference_checks`, for
  each referenced custom level with a proven available record whose `mlayer_id == Some(m)`,
  fire `frame-header/qm-mlayer-dependency-missing` unless
  `MLayerDependencyMap[obu_mlayer_id][m] == 1`, and `frame-header/qm-tlayer-dependency-missing`
  unless `TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][QmTLayerId] == 1`. A level reset to
  defaults (`QmMLayerId == -1`, `mlayer_id == None`) is exempt.
- New diagnostics `frame-header/qm-mlayer-dependency-missing` /
  `frame-header/qm-tlayer-dependency-missing` registered in `docs/VALIDATOR-DIAGNOSTICS.md`.

## Non-goals

- No change to the §7.3.8.9 availability check or the reset_qm presence arm (already landed).

## Acceptance criteria

- [ ] Both diagnostics registered and emitted from `frame_qm_reference_checks` only for levels
      with a proven record and `QmMLayerId >= 0`.
- [ ] Negative: a level defined at an undepended embedded layer fires
      `qm-mlayer-dependency-missing`; at an undepended temporal layer fires
      `qm-tlayer-dependency-missing`.
- [ ] Positive: a level at a depended (incl. reflexive base) layer is silent.
- [ ] `cargo xtask ci` passes.
