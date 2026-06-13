# validator delta: film-grain-layer-dependency-constraints

Advances `AV2-5.18.10-FILM-GRAIN-STRUCTURES` (and the `AV2-5.14-FILM-GRAIN`
reference-check note) by closing residual (b): the § 6.17.10.1 frame film-grain
layer-dependency constraints.

## ADDED Requirements

### Requirement: film-grain config layer-dependency constraints

The validator SHALL check the three § 6.17.10.1 bitstream-conformance requirements for a
frame `film_grain_config()` with `apply_grain == 1` that references an in-band film-grain
model slot `fgm_id`, against the active sequence header's § 5.4.1 dependency maps and
`chroma_format_idc` (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-10-1):
`MLayerDependencyMap[obu_mlayer_id][FgmMLayerId[fgm_id]] == 1`,
`TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][FgmTLayerId[fgm_id]] == 1`, and
`FgmChromaIdc[fgm_id] == chroma_format_idc`, where the model's stored layer identity and
chroma idc come from the § 5.14 film-grain OBU that defined the slot. The checks fire only
under `ExternalHlsMode::Disabled` (a film-grain model may be supplied by external means
under any Provided mode — the inexpressible-kind suppression), and only when the slot has an
in-band recorded model (an unavailable slot is owned by the
`frame-header/film-grain-model-unavailable` availability diagnostic).

#### Scenario: embedded-layer dependency missing

- **WHEN** a frame at `obu_mlayer_id` references a film-grain model recorded at an embedded
  layer the frame does not depend on (`MLayerDependencyMap[obu_mlayer_id][FgmMLayerId] == 0`)
- **THEN** an error diagnostic `frame-header/film-grain-mlayer-dependency-missing` (§ 6.17.10.1)
  is produced

#### Scenario: temporal-layer dependency missing

- **WHEN** a frame at `(obu_mlayer_id, obu_tlayer_id)` references a film-grain model recorded
  at a temporal layer the frame does not depend on
  (`TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][FgmTLayerId] == 0`)
- **THEN** an error diagnostic `frame-header/film-grain-tlayer-dependency-missing` (§ 6.17.10.1)
  is produced

#### Scenario: chroma idc mismatch

- **WHEN** a referenced film-grain model's stored `FgmChromaIdc[fgm_id]` differs from the
  active sequence header's `chroma_format_idc`
- **THEN** an error diagnostic `frame-header/film-grain-chroma-idc-mismatch` (§ 6.17.10.1)
  is produced

#### Scenario: satisfied constraints stay silent

- **WHEN** a frame depends on the model's embedded and temporal layers and the chroma idc
  matches
- **THEN** none of the three film-grain layer-dependency diagnostics are produced

#### Scenario: unavailable model is not layer-checked

- **WHEN** `apply_grain == 1` references a slot with no in-band recorded film-grain model
- **THEN** only `frame-header/film-grain-model-unavailable` is produced and the
  layer-dependency diagnostics stay silent

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
