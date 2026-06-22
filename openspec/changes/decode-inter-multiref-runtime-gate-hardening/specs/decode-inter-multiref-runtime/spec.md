## ADDED Requirements

### Requirement: Resolve PRIMARY_REF_CHOOSE before the cross-frame CDF-load reject
The decoder SHALL model AV2 § 5 `set_primary_ref_frame_and_ctx` (mirror
`docs/spec/av2/1.0.0/05-syntax-structures.md` :5411-5430) when deciding whether an
inter frame would load a prior frame's saved CDFs, INCLUDING the
`PRIMARY_REF_CHOOSE` resolution (mirror :5414-5415) via
`choose_primary_secondary_ref_frame` (mirror :5451-5510). The resolution loop SHALL
score ONLY reference slots whose `RefFrameType == INTER_FRAME` (mirror :5470), by
`qpDiff = Abs(RefBaseQIdx - base_q_idx)` with the `is_ref_better` order-hint
tie-break, so a key / intra-only reference history resolves an UNSIGNALLED
`PRIMARY_REF_CHOOSE` to `PRIMARY_REF_NONE` (no load). When
`signal_primary_ref_frame == 1`, the § 5 :5497-5508 tail SHALL override
`DerivedPrimaryRefFrame` to the signalled `primary_ref_frame` UNCONDITIONALLY
(regardless of the inter-only ranking result), so a signalled frame whose primary
names an adapted slot — even with no inter ranking candidate — loads that slot (and
is rejected below), rather than collapsing to `PRIMARY_REF_NONE` / no-load.

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

#### Scenario: a signalled primary overrides the ranking and loads its slot
- **WHEN** an inter frame has `signal_primary_ref_frame == 1` and a `primary_ref_frame`
  naming a retained slot for which the inter-only ranking finds no candidate
- **THEN** the signalled `primary_ref_frame` overrides `DerivedPrimaryRefFrame`, the
  frame loads `ref_frame_idx[primary_ref_frame]`, and it is rejected
  (`inter_cdf_inheritance_unmodeled`) when that slot is adapted — never silently
  decoded from default CDFs

#### Scenario: an out-of-range signalled primary reference is rejected
- **WHEN** an inter frame has `signal_primary_ref_frame == 1` and a `primary_ref_frame`
  that is a real reference (`< PRIMARY_REF_NONE`) but `>= NumTotalRefs` (out of
  `ref_frame_idx` bounds, non-conformant per § 6.19.2)
- **THEN** the decoder emits `decode/unsupported-feature`
  (`inter_primary_ref_out_of_range`) and produces no output, rather than decoding from
  default CDFs

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
BEFORE any output, an inter frame whose PARSED per-frame `use_ref_frame_mvs == 1`
(§ 5.18.2) once an INTER reference has been retained in the § 7.23 buffer, because the
buffer stores no § 7.23 `SavedMvs` and a § 7.12 temporal candidate would be predicted
from an empty (wrong) stack. A TMVP-capable sequence (`enable_ref_frame_mvs == 1`)
whose frame parsed `use_ref_frame_mvs == 0` draws no temporal candidate and is admitted.

#### Scenario: a temporal-MV frame over a retained inter reference is rejected
- **WHEN** a later inter frame has the parsed `use_ref_frame_mvs == 1` and an inter
  reference is already retained
- **THEN** the decoder emits `decode/unsupported-feature`
  (`inter_temporal_mvs_unmodeled`) and produces no output

### Requirement: Reject order-hint-wrapped reference histories
The decoder SHALL reject, with a structured `decode/unsupported-feature` diagnostic
BEFORE any output, an order-hint-wrapped reference history. It stores each slot's
`RefOrderHint` as the parsed `OrderHintLsbs`, which equals the unwrapped `OrderHint`
(`get_disp_order_hint()`, AV2 § 5.18.2 mirror :5368-5381) only while the history span
is small enough that the wrap correction never fires — the correction applies once
`maxDisp - OrderHintLsbs >= (1 << OrderHintBits) / 2` (HALF a window) — a DIRECTIONAL
wrap-back condition (a small LSB after larger prior hints). The reject therefore fires
ONLY when the max prior valid reference's order hint exceeds this frame's LSB by at
least HALF a `(1 << OrderHintBits)` window (a `monotonic_output_order_flag == 0`
wrap-back); a FORWARD frame (`LSB >= maxDisp`, any span) is exact and admitted. A
rejected wrap-back's stored `OrderHintLsbs` would differ from the unwrapped `OrderHint`
and mis-order the § 7.7 / `choose_primary_secondary_ref_frame` ranking.

#### Scenario: a wrap-back order-hint history is rejected
- **WHEN** the max prior valid reference's order hint exceeds the current frame's LSB
  by at least HALF an `OrderHintBits` window (e.g. a wrap-back: refs {0, 15}, next
  order hint 0, `OrderHintBits` 4)
- **THEN** the decoder emits `decode/unsupported-feature`
  (`inter_order_hint_wrapped`) and produces no output

### Requirement: Reject the §5 blend_cdfs secondary CDF load
The decoder SHALL reject, with a structured `decode/unsupported-feature` diagnostic
BEFORE any output, a loading inter frame that would invoke the AV2 § 5 :5431-5439
`blend_cdfs(ref_frame_idx[blendFrame])` secondary CDF load, because it models no
`blend_cdfs`. Inside the `load_cdfs` arm a conformant decoder ALSO blends when
`enable_avg_cdf == 1`, `avg_cdf_type == 0`, `blendFrame != PRIMARY_REF_NONE`, and
`!bru_inactive`. `blendFrame` is derived precisely (mirror :5432: it is
`derivedSecondaryRefFrame` when `primary_ref_frame == DerivedPrimaryRefFrame`, else
`DerivedPrimaryRefFrame`). Because `blend_cdfs(default, default) == default` is harmless
(== the minimal decoder's default), the reject fires ONLY when the resolved blend slot
itself ADAPTED (`disable_cdf_update == 0`). A loading frame with no `blendFrame`
(`PRIMARY_REF_NONE`: one INTER reference, unsignalled — the committed multi-reference
fixture) or whose blend slot did NOT adapt does NOT desync and SHALL stay admitted.

#### Scenario: a blend of an adapted secondary reference is rejected
- **WHEN** a loading inter frame's sequence has `enable_avg_cdf == 1` and
  `avg_cdf_type == 0` and its `blendFrame` resolves to a retained ADAPTED reference slot
- **THEN** the decoder emits `decode/unsupported-feature`
  (`inter_blend_cdf_unmodeled`) and produces no output

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
