## ADDED Requirements

### Requirement: Resolve PRIMARY_REF_CHOOSE before the cross-frame CDF-load reject
The decoder SHALL model AV2 § 5 `set_primary_ref_frame_and_ctx` (mirror
`docs/spec/av2/1.0.0/05-syntax-structures.md` :5411-5430) when deciding whether an
inter frame would load a prior frame's saved CDFs, INCLUDING the
`PRIMARY_REF_CHOOSE` resolution (mirror :5414-5415) via
`choose_primary_secondary_ref_frame` (mirror :5451-5510). The resolution loop SHALL
score ONLY reference slots whose `RefFrameType == INTER_FRAME` (mirror :5470), by
`qpDiff = Abs(RefBaseQIdx - base_q_idx)` with the `is_ref_better` order-hint
tie-break, so a key / intra-only reference history resolves
`PRIMARY_REF_CHOOSE` to `PRIMARY_REF_NONE` (no load).

The decoder SHALL reject, with a structured `decode/unsupported-feature` diagnostic
BEFORE any output, an inter frame whose RESOLVED `primary_ref_frame` loads a
reference slot whose saved CDFs were ADAPTED (`disable_cdf_update == 0`) while
`disable_cross_frame_cdf_init == 0` (the decoder does not model the § 7.23
cross-frame CDF save/load). A frame that resolves to `PRIMARY_REF_NONE`, to a
NON-adapted slot, or that has `disable_cross_frame_cdf_init == 1` SHALL NOT be
rejected on this basis.

#### Scenario: a CHOOSE frame resolving to an adapted inter slot is rejected
- **WHEN** an inter frame with `signal_primary_ref_frame == 0` resolves
  `PRIMARY_REF_CHOOSE` to a retained INTER reference whose `disable_cdf_update == 0`,
  with cross-frame CDF init enabled
- **THEN** the decoder emits `decode/unsupported-feature`
  (`inter_cdf_inheritance_unmodeled`) and produces no output

#### Scenario: a CHOOSE frame over a key-only history loads nothing
- **WHEN** an inter frame with `signal_primary_ref_frame == 0` has only a KEY frame
  as a valid reference
- **THEN** `choose_primary_secondary_ref_frame` resolves `PRIMARY_REF_CHOOSE` to
  `PRIMARY_REF_NONE`, the frame loads no cross-frame CDFs, and it is NOT rejected on
  the CDF-inheritance basis (the committed 2-frame inter fixtures, byte-identical)

### Requirement: Per-slot CDF adaptation tracking
The decoder SHALL record, per § 7.23 reference slot, whether the frame stored there
ADAPTED its CDFs (`disable_cdf_update == 0`), and SHALL key the cross-frame
CDF-load reject on the RESOLVED loaded slot's per-slot flag, not on a coarse "any
prior frame adapted" flag. A frame loading a NON-adapted slot SHALL be admitted even
when an unrelated earlier frame adapted.

#### Scenario: loading a non-adapted slot is admitted despite an earlier adapted frame
- **WHEN** an earlier frame adapted its CDFs but the frame under decode loads a
  DIFFERENT slot whose stored frame did not adapt
- **THEN** the decoder does not reject on the CDF-inheritance basis

### Requirement: Reject temporal-MV frames after retaining an inter reference
The decoder SHALL reject, with a structured `decode/unsupported-feature` diagnostic
BEFORE any output, an inter frame that uses temporal motion vectors
(`enable_ref_frame_mvs` or `use_ref_frame_mvs`) once an INTER reference has been
retained in the § 7.23 buffer, because the buffer stores no § 7.23 `SavedMvs` and a
§ 7.12 temporal candidate would be predicted from an empty (wrong) stack.

#### Scenario: a temporal-MV frame over a retained inter reference is rejected
- **WHEN** a later inter frame has `enable_ref_frame_mvs == 1` or
  `use_ref_frame_mvs == 1` and an inter reference is already retained
- **THEN** the decoder emits `decode/unsupported-feature`
  (`inter_temporal_mvs_unmodeled`) and produces no output

### Requirement: Reject order-hint-wrapped reference histories
The decoder SHALL store each slot's `RefOrderHint` as the unwrapped `OrderHint`
(`get_disp_order_hint()`, AV2 § 5.18.2 mirror :5368-5381) and SHALL reject, with a
structured `decode/unsupported-feature` diagnostic BEFORE any output, a reference
history whose distinct order hints span a full `(1 << OrderHintBits)` window — where
the stored `OrderHintLsbs` could differ from the unwrapped `OrderHint` and mis-order
the § 7.7 / `choose_primary_secondary_ref_frame` ranking.

#### Scenario: a wrapping order-hint history is rejected
- **WHEN** the prior valid slots' order hints plus the current frame's order hint
  span at least one `OrderHintBits` window
- **THEN** the decoder emits `decode/unsupported-feature`
  (`inter_order_hint_wrapped`) and produces no output

### Requirement: Require complete reference state before multi-reference ranking
The decoder SHALL keep the AV2 § 7.7 `derive_implicit_ref_map`
`valid_count > 1` → `UnmodeledDerivation` stop UNLESS all § 7.7 ranking inputs are
actually supplied: `RefBaseQIdx`, `RefOrderHint`, `RefFrameWidth`, and
`RefFrameHeight` as complete parallel slices covering every active reference slot,
plus the current frame size when the resolution-compatibility check (`check_res`) is
requested. A caller supplying a `None` or short slice for any ranking input SHALL
stop with `UnmodeledDerivation` rather than derive `ref_frame_idx` from defaulted
(fabricated zero) state.

#### Scenario: incomplete ranking inputs stop with UnmodeledDerivation
- **WHEN** two reference slots are valid but a ranking-input slice (e.g.
  `RefOrderHint`) is shorter than the active reference-slot count
- **THEN** `derive_implicit_ref_map` returns the `UnmodeledDerivation` stop, not a
  derived `ref_frame_idx`
