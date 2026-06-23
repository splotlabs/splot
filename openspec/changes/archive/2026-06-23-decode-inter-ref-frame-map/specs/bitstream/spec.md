# bitstream delta: decode-inter-ref-frame-map

Adds `AV2-7.7-GET-REF-FRAMES` (the implicit reference-map ranking) and advances
`AV2-5.18.2-FRAME-HEADER-INFO` (the non-intra control region) past the implicit
`get_ref_frames()` derivation for the at-most-one-valid-reference case.

## ADDED Requirements

### Requirement: implicit reference-map ranking (get_ref_frames)

The frame-header model SHALL provide a typed, total, panic-free `get_ref_frames()`
(§ 7.7) derivation computing `NumTotalRefs` and `ref_frame_idx[]` from explicit
per-slot reference state (`RefValid`, `RefOrderHint`, `RefBaseQIdx`, `RefCounter`,
`RefMLayerId`, `RefTLayerId`, `RefFrameWidth/Height`) and per-frame inputs
(`OrderHint`, `obu_mlayer_id`/`obu_tlayer_id`, `AllowedFrames`, the layer
dependency maps, `checkRes`). It SHALL implement the spec ranking exactly — the
`first_slot_with_ref` distinct-reference detection, the `valid_ref_frame_size`
resolution gate, the per-reference scoring (`Dist_Score_Lookup`, the `maxDisp` and
decay arms, the `refRatio` penalty, `tDist`), `get_relative_dist` (§ 5.18.3.1),
`new_score_or_dist`, the `get_unmapped_ref` over-selection drop, the
`bubble_sort_ref_scores` sort, the `NumTotalRefs = Min(NRanked,
ActiveNumRefFrames)` cut, and the trailing restricted-frame append — derived from
the § 7.7 spec text, never from AVM source.

#### Scenario: minimal single-reference post-key frame

- **WHEN** exactly one reference slot is valid (the post-CLK-key state) with the
  current frame's `OrderHint`
- **THEN** `get_ref_frames()` returns `NumTotalRefs == 1` and `ref_frame_idx ==
  [theSlot]` for both `checkRes == 0` and `checkRes == 1`

#### Scenario: resolution gate drops an incompatible reference on the second call

- **WHEN** a single valid reference is outside the current frame's § 7.7
  resolution window
- **THEN** the `checkRes == 0` call admits it but the `checkRes == 1` call drops
  it (`NumTotalRefs == 0`)

#### Scenario: distinct references rank by score

- **WHEN** two distinct valid references are present
- **THEN** `ref_frame_idx` lists them in ascending § 7.7 score order

### Requirement: implicit reference map in the inter control region

The § 5.18.2 inter control parser SHALL consult the `get_ref_frames()` model at
its `get_ref_frames(0)` (mirror :4607) and `get_ref_frames(1)` (mirror :4647)
call sites, GATED to the at-most-one-valid-reference case the modeled reference
state can resolve exactly. On that gated path it SHALL advance past
`InterStop::UnmodeledDerivation` and parse the inter control region to the shared
tail (`InterStop::ReachedSharedTail`). A richer reference state (two or more
proven-valid slots, or no modeled reference view) SHALL keep the honest
`InterStop::UnmodeledDerivation` stop with facts preserved. This is parse-level
only; no decode output changes.

#### Scenario: implicit-map inter frame reaches the shared tail

- **WHEN** the minimal `OBU_CLOSED_LOOP_KEY` key + `OBU_REGULAR_TILE_GROUP` inter
  fixture's inter frame is parsed with the post-key reference state (one valid
  slot)
- **THEN** the inter control region reaches `InterStop::ReachedSharedTail` with
  `NumTotalRefs == 1` and `ref_frame_idx == [0]`

#### Scenario: ambiguous reference state stays unmodeled

- **WHEN** the modeled reference state proves two or more valid slots
- **THEN** the implicit map stops with `InterStop::UnmodeledDerivation` (no
  guessing the § 7.7 scoring inputs the model does not carry)
