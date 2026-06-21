# Proposal: model § 7.7 get_ref_frames() (the implicit reference map)

## Feature IDs

- `AV2-7.7-GET-REF-FRAMES` (new — the implicit reference-map ranking process)
- `AV2-5.18.2-FRAME-HEADER-INFO` (the non-intra control region that consumes it)

## Why

The § 5.18.2 inter control parser
(`crates/splot-core/src/headers/frame/inter.rs`) stops with
`InterStop::UnmodeledDerivation` on any frame using the IMPLICIT reference map
(`explicitRefFrameMap == 0`): it cannot continue because `get_ref_frames()`
(§ 7.7) determines `NumTotalRefs` / `ref_frame_idx[]` and hence every later
inter-header bit position (`CeilLog2(NumTotalRefs)` for `bru_ref`, the
`use_ref_frame_mvs` / TIP gates that test `NumTotalRefs`, …). The committed
dual-oracle minimal inter fixture
(`tests/conformance/vectors/valid/syn-key-inter-64x64.ivf`, a
`OBU_CLOSED_LOOP_KEY` key + `OBU_REGULAR_TILE_GROUP` inter frame) uses the
implicit path, so this derivation is the inter arc's blocker.

## What Changes

1. Add a typed, total, panic-free § 7.7 model
   (`crates/splot-core/src/headers/frame/get_ref_frames.rs`,
   `get_ref_frames`) implementing the full ranking on explicit per-slot inputs:
   `first_slot_with_ref` (RefValid + RefCounter dedup), `valid_ref_frame_size`
   (the resolution window), the per-reference scoring (the § 3
   `Dist_Score_Lookup[7]` table, the `maxDisp` / decay arms with
   `DIST_WEIGHT_BITS == 6`, `DECAY_DIST_CAP == 6`, the `refRatio` penalty,
   `tDist`), `get_relative_dist` (§ 5.18.3.1), `new_score_or_dist`,
   `get_unmapped_ref` (the `NRanked > REFS_PER_FRAME` drop), the exact spec
   `bubble_sort_ref_scores`, the `NumTotalRefs = Min(NRanked,
   ActiveNumRefFrames)` cut, and the trailing restricted-frame append. Derived
   from the § 7.7 spec text, never from AVM source.
2. Wire it into the § 5.18.2 inter control parser's `get_ref_frames(0)`
   (mirror :4607) and `get_ref_frames(1)` (mirror :4647) calls, GATED to the
   at-most-one-valid-reference case the modeled `FrameReferenceStateView`
   (RefValid / RefOrderHint / dims) can resolve EXACTLY. With ≤ 1 proven-valid
   slot the score/sort/drop machinery is irrelevant, so the result is the exact
   § 7.7 answer; a richer reference state (≥ 2 valid slots, or no modeled view)
   stays an honest `InterStop::UnmodeledDerivation` stop.
3. Unit-test § 7.7 against worked examples (the minimal post-key single-ref
   frame, distinct-ref ranking, the `ActiveNumRefFrames` cap, restricted-frame
   append, the resolution gate, `AllowedFrames` masking, `get_relative_dist`
   sentinels) and prove on the REAL fixture bytes that the inter header now
   reaches `InterStop::ReachedSharedTail`.

This is PARSE-LEVEL ONLY: no decode output changes (the inter frame still won't
reconstruct — `mode_info` / MV / motion-compensation are later bricks). The
verification is the unit tests.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `bitstream`: Add the § 7.7 implicit reference-map ranking model and consume it
  in the § 5.18.2 inter control region so the implicit-map minimal inter frame
  advances past `InterStop::UnmodeledDerivation` to the shared tail, gated to the
  at-most-one-valid-reference case the modeled reference state resolves exactly.

## Impact

- `crates/splot-core/src/headers/frame/get_ref_frames.rs` (new module).
- `crates/splot-core/src/headers/frame/inter.rs` (the implicit-map wiring +
  `InterFrameContext::order_hint`; the § 7.7 control-level and fixture-bytes
  parse tests).
- `crates/splot-core/src/headers/frame/info.rs` (thread `order_hint` into the
  inter context).
- `crates/splot-core/src/headers/frame/mod.rs` (register the module).
- Tracking docs: `docs/IMPLEMENTATION-MATRIX.toml` (new
  `AV2-7.7-GET-REF-FRAMES` row + `AV2-5.18.2-FRAME-HEADER-INFO` notes) and the
  generated `docs/FEATURE-STATUS.md` / `docs/SPEC-COVERAGE.md`.
- No new dependencies; no decode-output change; no public encoder surface.
