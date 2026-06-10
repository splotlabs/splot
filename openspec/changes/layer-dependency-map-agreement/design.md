# Design — layer-dependency-map agreement checks

## Context

PR #35's prerequisite landed earlier: `SequenceHeaderGeneral` exposes the
derived § 5.4.1 maps as opaque newtypes queried via
`MLayerDependencyMap::depends_on(curr, ref)` and
`TLayerDependencyMap::depends_on(mlayer, curr, ref)`
(`crates/splot-core/src/headers/sequence.rs`; out-of-range ids read `false`).
Three normative agreement requirements consume those maps:

- **§ 6.10.7** (mirror `06-syntax-structures-semantics.md#s-6-10-7`, lines
  3028-3042): explicitly signalled `ops_mlayer_map` / `ops_tlayer_map`, "if
  present, shall agree with the indication in the information in the activated
  sequence header" — for any set bit `cMId` with
  `MLayerDependencyMap[cMId][rMId] == 1`, bit `rMId` must also be set for all
  non-negative `rMId < cMId`; analogously per embedded layer `cMId` for
  `ops_tlayer_map` bits `cTId`/`rTId` under `TLayerDependencyMap[cMId][cTId][rTId]`.
- **§ 6.8.9** (same file, `#s-6-8-9`, lines 1988-2009): identical closure
  shape for `lcr_mlayer_map[isGlobal][xId]` and
  `lcr_tlayer_map[isGlobal][xId][cMId]` "in the activated LCR OBU … shall
  agree with the equivalent indication in the activated sequence header", with
  four bullets covering `isGlobal` 0/1 × mlayer/tlayer.
- **§ 7.3.8.7** (mirror `07-decoding-process.md#s-7-3-8-7`): "the layer
  dependency constraints TLayerDependencyMap and MLayerDependencyMap are
  satisfied for the referenced multi-frame header OBU". The concrete predicate
  is § 6.17.2 (`06-syntax-structures-semantics.md#s-6-17-2`, lines 4345-4351):
  after `load_sequence_header`, when `cur_mfh_id > 0`,
  `MLayerDependencyMap[obu_mlayer_id][MfhMLayerId[cur_mfh_id]] == 1` and
  `TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][MfhTLayerId[cur_mfh_id]] == 1`.

Current validator state (all in `crates/splot-validate/src/context.rs`):

- OPS: `check_operating_point_set_semantics` runs per OPS OBU;
  `OperatingPointSetRecord` keeps only `{xlayer_id, ops_id, ops_cnt, offset}`
  — the parsed `OpsMlayerInfo` maps are dropped.
- LCR: the HLS store keeps only `global_id -> lcr_xlayer_map` and
  `(xlayer, local_id)` existence — the parsed `LcrEmbeddedLayerInfo` is
  dropped. `check_seq_lcr_reference` already resolves `seq_lcr_id`
  local-first-then-global (§ 6.4.1).
- MFH: `MultiFrameHeaderRecord` already carries `mfh_tlayer_id` /
  `mfh_mlayer_id` (= `MfhTLayerId` / `MfhMLayerId`) and `mfh_seq_header_id`;
  the check site is the `TODO(spec: AV2-5.7-MULTI-FRAME-HEADER)` in
  `resolve_frame_header_reference`'s `cur_mfh_id > 0` branch.
- Activation: `active_sequence_by_xlayer` is set at two points — OBU-order
  fallback (`or_insert` in `observe_sequence_header`) and the § 5.18.2
  frame-header override in `observe_frame_bearing_obu`.

## Goals / Non-Goals

**Goals:**

- Emit the six new error diagnostics with zero false positives: a check runs
  only when every input (activated sequence header, resolved LCR/OPS/MFH
  state) is modeled in-band, and is suppressed under
  `ExternalHlsMode::Provided` when an external sequence header could be the
  activated one (precedent: `validate_active_sequence_limits`).
- Cover both OBU orderings (HLS-before-activation and HLS-after-activation)
  for the OPS and LCR checks without duplicate diagnostics.
- Preserve every existing diagnostic unchanged.

**Non-Goals:** see the proposal (MFH frame-size bounds, Annex A/E, decoder-
ignore reserved-value rules, `lcr_max_expected_*` bounds, external-HLS
modeling).

## Decisions

### D1: Shared closure helper over `depends_on`

One private helper translates the spec predicate once for both bitmask checks
(OPS and LCR have the same closure shape):

- mlayer: for each `cMId` with map bit set, for each `rMId < cMId` where
  `m_map.depends_on(cMId, rMId)`, require bit `rMId` set; report the first
  missing `(cMId, rMId)` pair per map (one diagnostic per violating map, with
  the offending pair in the message — mirrors how existing checks report one
  diagnostic per syntax element rather than per bit).
- tlayer: for each embedded layer `cMId` carrying a tlayer map, for each set
  `cTId`, for each `rTId < cTId` where `t_map.depends_on(cMId, cTId, rTId)`,
  require bit `rTId` set.

The MFH check is a direct two-predicate test, not a closure, and stays inline
at the frame-header reference site.

### D2: When each check runs (activation pairing)

- **MFH (`frame-header/mfh-*`):** in `resolve_frame_header_reference`, after
  the `MultiFrameHeaderRecord` resolves and its `mfh_seq_header_id` resolves
  **in-band** (that is the validator's `load_sequence_header`). Emitted per
  violating frame-header OBU — per-OBU diagnostics need no dedup (precedent:
  `sequence-state/tlayer-exceeds-max`).
- **OPS (`ops/*-dependency-missing`):** two-sided —
  (a) at OPS observation, each payload entry with **explicitly** signalled
  mlayer info is checked against the sequence header currently activated for
  that entry's extended layer (skip when none);
  (b) when `active_sequence_by_xlayer` for an xlayer is newly set or changes
  to a different id, stored explicit maps of active OPS records relevant to
  that xlayer (local OPS in that xlayer bucket; global-OPS entries for that
  xlayer) are re-checked against the newly activated header.
  Rationale: global HLS commonly precedes the first frame of a temporal unit,
  so observation-time-only would miss the canonical `[TD, SH, OPS, frame]`
  layout; activation-time-only would miss OPS OBUs arriving after activation.
- **LCR (`lcr/*-dependency-missing`):** activation-time only. When the
  activated sequence header for xlayer `x` has `seq_lcr_id != 0` and it
  resolves in-band (local-first-then-global, reusing the
  `check_seq_lcr_reference` resolution order), the resolved record's
  embedded-layer info **for entry `xId == x`** is checked against that
  header's maps. A later-arriving LCR is deliberately NOT retroactively
  paired with an earlier activation: § 6.4.1 associates only an LCR "present
  prior to this sequence header" (a stream that needs the late LCR is already
  flagged by the § 7.3.8.3 availability check). An LCR that never pairs with
  an activated sequence header is not checked (§ 6.8.9 binds "the activated
  LCR OBU" to "the activated sequence header"), and a redefinition replaces
  the stored maps wholesale so the checks only ever see the latest
  definition. Gate: unlike the OPS checks, suppressed under **any**
  `ExternalHlsMode::Provided` — external HLS cannot declare LCRs, and an
  unmodeled external local LCR would win the § 6.4.1 resolution (same
  rationale as the `lcr/global-xlayer-map-missing-xlayer` gate).

### D3: Dedup for activation-driven checks

OPS side (b) and the LCR check can re-fire as frames re-activate headers. A
`BTreeSet` of emitted keys in `ValidatorContext` makes each finding fire once:

- OPS key: `(ops OBU offset, payload index, entry xlayer, seq_header_id, which-map)`.
- LCR key: `(xlayer, seq_header_id, lcr identity (is_global + id + defining
  OBU offset), which-map)` — the offset makes a redefined LCR a distinct
  violating object.

Re-checks run only when the active id for the xlayer actually changes
(`None -> id` or `id1 -> id2`), with the set as backstop for `A -> B -> A`
flapping. A different activated header *id* producing a new disagreement gets
a distinct key; a same-id sequence-header **redefinition** that changes the
agreement inputs (dependency maps or `seq_lcr_id` — legal at a CVS boundary,
§ 7.3.6) instead invalidates that id's emitted keys and re-runs the checks for
every extended layer the id is active for, so reconfiguration is still caught
and re-reported against the new content.

### D4: State storage

- `OperatingPointSetRecord` grows
  `explicit_entries: Vec<OpsRecordEntry { payload_index, xlayer_id, mlayer_map, tlayer_maps }>`
  (clone of the parsed `OpsMlayerInfo`, explicit entries only — § 6.10.7 binds
  "if present"; `Inherited` references are checked when the *referenced* OPS
  is itself observed, `Absent` has nothing to check). The § 6.10.1
  reset/update semantics in `OpsAvailabilityStore` apply unchanged.
- The HLS store's LCR records grow the parsed embedded-layer maps:
  `global_id -> {xlayer_map, per-xId embedded info}` and
  `(xlayer, local_id) -> embedded info`, where embedded info is
  `mlayer_map: u8` plus per-set-bit `tlayer_map`s (from
  `LcrEmbeddedLayerInfo` / `LcrEmbeddedLayer`). Stored at the existing
  recording points (already gated on full parse + valid § 5.2.1 tail).
- No `splot-core` API changes required: `depends_on` and the parsed models
  cover everything (dependency direction preserved; no new dependencies).

### D5: Rule ids, severities, spec sections

| Rule id | Severity | `spec_section` |
|---|---|---|
| `ops/mlayer-dependency-missing` | error | `6.10.7` |
| `ops/tlayer-dependency-missing` | error | `6.10.7` |
| `lcr/mlayer-dependency-missing` | error | `6.8.9` |
| `lcr/tlayer-dependency-missing` | error | `6.8.9` |
| `frame-header/mfh-mlayer-dependency-missing` | error | `7.3.8.7` |
| `frame-header/mfh-tlayer-dependency-missing` | error | `7.3.8.7` |

The first two ids are the roadmap-planned names (VALIDATOR-ROADMAP.md backlog
rows). The LCR pair mirrors them. The MFH pair lives in the `frame-header/`
namespace because the violating OBU is the frame header referencing the MFH
(§ 6.17.2 evaluates the constraint at the frame); messages cite the § 6.17.2
predicate. All three namespaces already exist in `DIAGNOSTIC_PREFIXES`; only
registry rows are added. Byte offset: the violating OBU (OPS OBU, frame-header
OBU; for LCR, the activating frame/sequence-header OBU is *not* the violator —
the diagnostic carries the **LCR OBU's** recorded offset).

### D6: Spec-interpretation notes (recorded for review)

- § 6.10.7/§ 6.8.9 bind each per-xlayer entry (`xLId`/`xId`) to "the activated
  sequence header"; this design reads that as the sequence header activated
  for that extended layer (`active_sequence_by_xlayer[xLId]`), matching the
  per-xlayer activation model the validator already uses for
  `validate_active_sequence_limits` and `metadata_applies_to`.
- The § 6.10.7 lead-in spells `MlayerDependencyMap`/`TlayerDependencyMap`
  (lowercase 'l') and writes `TlayerDependencyMap[cMId][cTId][cTId]` where the
  bullets use `[cMId][cTId][rTId]`; the bullets are taken as the operative
  predicate (the lead-in's third index is an apparent typo for `rTId`).
- Unlike `metadata_applies_to`, the checks never fall back to
  `default_for`-derived maps: a fabricated map could flag a conformant stream.
  No activated in-band header ⇒ no check.

## Risks / Trade-offs

- [Missed violations when no sequence header activates for an xlayer] →
  Accepted: soundness over completeness, consistent with the repo's
  no-false-positive principle; the deferred-non-check note in
  VALIDATOR-DIAGNOSTICS.md is replaced by precise wording of what is checked.
- [Missed violations when an LCR is redefined between activations] →
  Accepted: the redefined maps are evaluated at the next activation event
  only; re-checking at LCR observation would retroactively pair a record
  § 6.4.1 never associates.
- [External HLS suppression hides real errors when external headers are
  declared] → Same trade-off the existing `sequence-state/*` checks make;
  exact enforcement needs external sequence-header content modeling
  (`TODO(spec: AV2-7.3.8-HLS-AVAILABILITY)` unchanged).
- [State growth: cloned OPS/LCR maps] → Bounded and small (≤ 8-bit masks ×
  ≤ 8 layers × ≤ 31 xlayers per record); same lifetime as the existing
  records.
- [Duplicate diagnostics across re-activations] → D3 dedup keys; tests cover
  the `A -> B -> A` and repeated-frame cases.
- [§ 6.17.2 also constrains MFH frame-size overrides] → Explicit non-goal;
  the matrix note for `AV2-5.7-MULTI-FRAME-HEADER` records it as remaining
  future work so the row stays honest.

## Open Questions

None blocking. The D6 interpretation notes are called out for reviewer
attention in the PR.
