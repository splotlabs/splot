# validator delta — layer-dependency-map agreement checks

## ADDED Requirements

### Requirement: OPS dependency-map agreement

`splot-validate` SHALL check explicitly signalled `ops_mlayer_map` /
`ops_tlayer_map` entries for dependency closure under the activated sequence
header's `MLayerDependencyMap` / `TLayerDependencyMap` (AV2 v1.0.0 § 6.10.7):
for any embedded layer `cMId` included by `ops_mlayer_map` whose
`MLayerDependencyMap[cMId][rMId]` is 1, bit `rMId` SHALL also be included for
all non-negative `rMId < cMId`, and for any temporal layer `cTId` included by
`ops_tlayer_map[..][cMId]` whose `TLayerDependencyMap[cMId][cTId][rTId]` is 1,
bit `rTId` SHALL also be included for all non-negative `rTId < cTId`. Each
per-extended-layer entry is checked against the sequence header activated for
that entry's extended layer, both when the OPS OBU is observed and when a
later activation makes the pairing decidable, without duplicate diagnostics
for the same `(OPS instance, entry, sequence header)` pairing. Inherited and
absent mlayer info SHALL NOT be checked (§ 6.10.7 binds the maps "if
present").

#### Scenario: OPS mlayer map missing a required dependency

- **GIVEN** an activated sequence header whose `MLayerDependencyMap[1][0]` is 1
- **AND** an OPS entry for that extended layer whose `ops_mlayer_map` includes
  embedded layer 1 but not embedded layer 0
- **WHEN** the validator runs
- **THEN** it SHALL emit an `ops/mlayer-dependency-missing` error (§ 6.10.7).

#### Scenario: OPS tlayer map missing a required dependency

- **GIVEN** an activated sequence header whose
  `TLayerDependencyMap[0][1][0]` is 1
- **AND** an OPS entry for that extended layer whose `ops_tlayer_map` for
  embedded layer 0 includes temporal layer 1 but not temporal layer 0
- **WHEN** the validator runs
- **THEN** it SHALL emit an `ops/tlayer-dependency-missing` error (§ 6.10.7).

#### Scenario: dependency-closed OPS maps are silent

- **GIVEN** an activated sequence header and an OPS whose explicit
  `ops_mlayer_map` / `ops_tlayer_map` entries are dependency-closed under its
  maps
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `ops/*-dependency-missing` diagnostic.

#### Scenario: OPS before the first activation is still checked

- **GIVEN** a temporal unit carrying a sequence header, then a global OPS,
  then a frame header that activates that sequence header for the entry's
  extended layer
- **AND** the OPS maps disagree with the activated header's maps
- **WHEN** the validator runs
- **THEN** it SHALL emit the corresponding `ops/*-dependency-missing` error
  exactly once for that pairing.

#### Scenario: no activated sequence header means no OPS agreement check

- **GIVEN** an OPS entry for an extended layer with no in-band activated
  sequence header
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `ops/*-dependency-missing` diagnostic for
  that entry (the maps are never fabricated from defaults).

#### Scenario: external sequence headers suppress the OPS agreement check

- **GIVEN** validation runs with `ExternalHlsMode::Provided` declaring at
  least one sequence header
- **WHEN** an OPS entry's maps disagree with the in-band activated header
- **THEN** the validator SHALL NOT emit an `ops/*-dependency-missing` error
  (an externally activated header with unmodeled maps may govern).

#### Scenario: a same-id sequence-header redefinition re-binds the checks

- **GIVEN** an OPS finding emitted against sequence header id `N`
- **AND** a later sequence header reusing id `N` whose agreement inputs
  (dependency maps or `seq_lcr_id`) changed while the stored OPS maps still
  disagree
- **WHEN** the validator runs
- **THEN** it SHALL re-emit the finding against the redefined content (the
  id's dedup keys are invalidated by the redefinition).

### Requirement: LCR dependency-map agreement

`splot-validate` SHALL check the activated LCR's
`lcr_mlayer_map[isGlobal][xId]` / `lcr_tlayer_map[isGlobal][xId][cMId]` for
the same dependency closure under the activated sequence header's
`MLayerDependencyMap` / `TLayerDependencyMap` (AV2 v1.0.0 § 6.8.9, all four
`isGlobal` × map bullets). The pairing is per extended layer and evaluated at
activation events only: when the sequence header activated for xlayer `x`
resolves `seq_lcr_id` in-band (local-first-then-global, § 6.4.1), the resolved
record's embedded-layer info for `xId == x` — as stored by its latest
definition — is checked against that header's maps, once per
`(xlayer, sequence header, defining LCR OBU)` pairing. An LCR arriving after
the activating sequence header SHALL NOT be retroactively paired (§ 6.4.1
associates only an LCR "present prior to this sequence header"), and the check
SHALL be suppressed whenever external HLS is enabled (an unmodeled external
local LCR would win the § 6.4.1 resolution). The diagnostics SHALL carry the
LCR OBU's byte offset.

#### Scenario: activated local LCR mlayer map missing a required dependency

- **GIVEN** an activated sequence header for xlayer `x` with
  `seq_lcr_id != 0` resolving to an in-band local LCR in xlayer `x`
- **AND** `MLayerDependencyMap[1][0]` is 1 while the LCR's
  `lcr_mlayer_map[0][x]` includes embedded layer 1 but not embedded layer 0
- **WHEN** the validator runs
- **THEN** it SHALL emit an `lcr/mlayer-dependency-missing` error (§ 6.8.9)
  at the LCR OBU's offset.

#### Scenario: activated global LCR tlayer map missing a required dependency

- **GIVEN** an activated sequence header for xlayer `x` whose `seq_lcr_id`
  resolves to an in-band global LCR whose `lcr_xlayer_map` includes `x`
- **AND** `TLayerDependencyMap[0][1][0]` is 1 while the LCR's
  `lcr_tlayer_map[1][x][0]` includes temporal layer 1 but not temporal layer 0
- **WHEN** the validator runs
- **THEN** it SHALL emit an `lcr/tlayer-dependency-missing` error (§ 6.8.9).

#### Scenario: dependency-closed activated LCR is silent

- **GIVEN** an activated sequence header whose `seq_lcr_id` resolves to an
  in-band LCR whose maps for that xlayer are dependency-closed under the
  header's maps
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `lcr/*-dependency-missing` diagnostic.

#### Scenario: unactivated or unresolved LCR pairings are not checked

- **GIVEN** an LCR that no activated sequence header resolves via
  `seq_lcr_id`, or a sequence header with `seq_lcr_id == 0`
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `lcr/*-dependency-missing` diagnostic, and
  SHALL NOT emit duplicates when the same pairing re-activates across frames.

#### Scenario: a later LCR is not retroactively paired

- **GIVEN** a sequence header with `seq_lcr_id != 0` followed (not preceded)
  by an LCR with that id whose maps disagree
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `lcr/*-dependency-missing` diagnostic (the
  § 7.3.8.3 availability diagnostic owns the stream).

#### Scenario: provided external HLS suppresses the LCR agreement check

- **GIVEN** validation runs with `ExternalHlsMode::Provided` (even an empty
  set)
- **WHEN** an in-band resolved LCR's maps disagree with the activated header
- **THEN** the validator SHALL NOT emit any `lcr/*-dependency-missing`
  diagnostic.

#### Scenario: a redefinition replaces the checked maps

- **GIVEN** a violating LCR followed by a redefinition of the same id without
  embedded-layer info, then the activating sequence header
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `lcr/*-dependency-missing` diagnostic (the
  latest definition has nothing to check).

### Requirement: Frame-header MFH layer-dependency checks

For a parsed frame-header prefix with `cur_mfh_id > 0` whose multi-frame
header and the MFH's `mfh_seq_header_id` both resolve in-band,
`splot-validate` SHALL enforce the § 7.3.8.7 layer-dependency constraints
using the § 6.17.2 predicate evaluated after the sequence header is loaded:
`MLayerDependencyMap[obu_mlayer_id][MfhMLayerId[cur_mfh_id]]` SHALL be 1 and
`TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][MfhTLayerId[cur_mfh_id]]`
SHALL be 1, where `obu_mlayer_id` / `obu_tlayer_id` are the frame header's and
`MfhMLayerId` / `MfhTLayerId` are the recorded multi-frame header's. This
resolves the deferred `TODO(spec: AV2-5.7-MULTI-FRAME-HEADER)` check.

#### Scenario: frame does not depend on the MFH's embedded layer

- **GIVEN** a frame header with `cur_mfh_id > 0` resolving to an MFH recorded
  with `MfhMLayerId` equal to `m`
- **AND** the loaded sequence header's
  `MLayerDependencyMap[obu_mlayer_id][m]` is 0
- **WHEN** the validator observes the frame-header prefix
- **THEN** it SHALL emit a `frame-header/mfh-mlayer-dependency-missing` error
  (§ 7.3.8.7) at the frame-header OBU's offset.

#### Scenario: frame does not depend on the MFH's temporal layer

- **GIVEN** a frame header with `cur_mfh_id > 0` resolving to an MFH recorded
  with `MfhTLayerId` equal to `t`
- **AND** the loaded sequence header's
  `TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][t]` is 0
- **WHEN** the validator observes the frame-header prefix
- **THEN** it SHALL emit a `frame-header/mfh-tlayer-dependency-missing` error
  (§ 7.3.8.7).

#### Scenario: satisfied MFH layer dependencies are silent

- **GIVEN** a frame header whose `cur_mfh_id` resolves to an MFH whose
  recorded layer ids satisfy both § 6.17.2 dependency-map predicates under the
  loaded sequence header
- **WHEN** the validator observes the frame-header prefix
- **THEN** it SHALL NOT emit any `frame-header/mfh-*-dependency-missing`
  diagnostic.

#### Scenario: unresolved MFH or sequence header is not layer-checked

- **GIVEN** a frame header with `cur_mfh_id > 0` whose MFH is unavailable, or
  whose MFH's sequence header resolves only externally or not at all
- **WHEN** the validator observes the frame-header prefix
- **THEN** it SHALL NOT emit any `frame-header/mfh-*-dependency-missing`
  diagnostic (the existing availability diagnostics own those cases).
