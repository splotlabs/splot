# Design: decode-inter-ref-frame-map

## Context

`get_ref_frames()` (AV2 § 7.7,
`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-7`) is the implicit
reference-map ranking. § 5.18.2 calls it twice on the `!explicitRefFrameMap`
path: `get_ref_frames(0)` (mirror :4607, before `frame_size()`) and
`get_ref_frames(1)` (mirror :4647, after, with the resolution gate +
restricted-frame append). It reads no bits, but its `NumTotalRefs` /
`ref_frame_idx[]` outputs determine every later inter-header bit position.

The inputs § 7.7 reads (§ 7.23 reference state): `RefValid`, `RefOrderHint`,
`RefBaseQIdx`, `RefCounter`, `RefMLayerId`, `RefTLayerId`, `RefFrameWidth/Height`,
plus per-frame `OrderHint`, `obu_mlayer_id`/`obu_tlayer_id`, `AllowedFrames`
(§ 5.18.2 :4539), and the `TLayerDependencyMap` / `MLayerDependencyMap`.

## Decisions

### 1. Standalone, fully-modeled § 7.7 on explicit inputs

`get_ref_frames.rs` implements the COMPLETE § 7.7 algorithm on a typed
`GetRefFramesInput` (per-slot `RefSlot` + per-frame scalars + a
layer-dependency predicate). This keeps the model spec-faithful and unit-testable
independent of how much reference state the parser threads. Constants come from
the § 3 symbol table (`REFS_PER_FRAME == 7`, `RESTRICTED_OH == -1`,
`DIST_WEIGHT_BITS == 6`, `DECAY_DIST_CAP == 6`) and the § 7.7
`Dist_Score_Lookup[7]`. The bubble sort matches the spec's exact comparison
(`ScoresScore[j] > ScoresScore[j+1]`) so equal-score ordering is the spec's.

### 2. Gate the parser wiring to the at-most-one-valid-reference case

`FrameReferenceStateView` carries only `RefValid` / `RefOrderHint` / dims today
— NOT `RefBaseQIdx` / `RefCounter` / layer ids / `AllowedFrames` / the dependency
maps. Those feed § 7.7's score / sort / drop / restricted machinery. When the
modeled view proves **at most one** valid slot, that machinery is irrelevant: the
result is `NumTotalRefs = Min(NRanked, ActiveNumRefFrames)` over a single distinct
reference, i.e. `[theSlot]` (one valid) or `[]` (none). That outcome is
INDEPENDENT of every unmodeled score input, so `derive_implicit_ref_map` builds a
`GetRefFramesInput` with deterministic single-spatial-layer defaults (distinct
`RefCounter` per slot, `AllowedFrames == -1`, layer-dependency maps == 1) and
returns the EXACT § 7.7 answer — the real ranking, proven, not a hardcoded `[0]`.
A view with ≥ 2 valid slots (or no modeled `RefValid`) returns `None` and the
parser keeps the honest `InterStop::UnmodeledDerivation` stop.

Why this is the minimal fixture's regime: a `OBU_CLOSED_LOOP_KEY` key with
`refresh_frame_flags == 255` leaves ONLY slot 0 `RefValid` after the § 7.23
`first` rule (mirror :14132, `(KEY) ? first : 1` — slot 0 gets `first == 1`, the
rest `0`). So the post-key inter frame sees exactly one valid slot.

### 3. Faithful two-call sequence

Both § 7.7 calls run: `checkRes == 0` before `frame_size`, `checkRes == 1` after,
overwriting `NumTotalRefs` / `ref_frame_idx`. On the gated path the second call
applies `valid_ref_frame_size` (and would append restricted frames), so the model
reproduces the spec's two-call sequence rather than assuming it is a no-op — a
resolution-incompatible single reference is correctly dropped on the second call
(unit-tested).

## Honesty

- The derivation is REAL: with one valid slot the model still runs
  `first_slot_with_ref` / scoring / the `Min(NRanked, ActiveNumRefFrames)` cut and
  PRODUCES `[0]`; it is not short-circuited to a literal.
- No decode output changes. The inter frame still does not reconstruct; the
  verification is the unit tests (worked examples + the fixture-bytes
  shared-tail proof).
- AVM (`av2/common/pred_common.c`) was read ONLY to confirm the § 3 constants /
  score table match the spec; no AVM source, table, comment, or prose was copied.

## Deferred

- The ≥ 2-valid-reference ranking needs `FrameReferenceStateView` (and the
  validator's § 7.23 model) extended with `RefBaseQIdx` / `RefCounter` /
  `RefMLayerId` / `RefTLayerId` / `AllowedFrames` / the dependency maps.
- The inter shared tail past `ReachedSharedTail` and the § 5.20 inter tile body.
- The inspector's `unknown()` view (no cross-OBU state) means standalone
  `splot inspect` still shows the `UnmodeledDerivation` stop; the modeled advance
  is realized in the validator path and the unit tests.
