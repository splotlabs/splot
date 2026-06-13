# Change: inter-ref-frame-scale-ratio

## Feature IDs

- `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS`
- `AV2-5.18.2-FRAME-HEADER-INFO`

## Why

The inter frame-header reference-state path already models the §7.23 reference-frame buffer
(`ReferenceStateTracker` / `SlotFacts`) and most §6.17.2 reference checks
(`num_total_refs`/`primary_ref_frame`/`bru_ref`/`ref_frame_idx` validity, RAS long-term-id).
One normative §6.17.2 constraint that is decidable from already-modeled frame-header state
remains unimplemented: the **reference-scaling ratio**
(docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2 :4638-4644):

> Once the frame size has been determined, it is a requirement of bitstream conformance that
> all the following conditions are satisfied for i=0..NumTotalRefs-1:
> 2*FrameWidth >= RefFrameWidth[ref_frame_idx[i]], 2*FrameHeight >= RefFrameHeight[...],
> FrameWidth <= 16*RefFrameWidth[...], FrameHeight <= 16*RefFrameHeight[...].

i.e. an inter frame may upscale a reference by at most 2x and downscale it by at most 16x on
each axis. Both operands are already modeled: the current frame's size on `core.frame_size`
(resolved by the inter `frame_size_with_refs()` / `frame_size()` parse) and the reference
slot's stored dims in the §7.23 `SlotFacts.{width,height}`. The check is the natural keystone
of the next Tier-1 slice because it exercises the join of those two modeled pillars against a
real normative rule, needs no new parser/types/modules, and is provably zero-false-positive.

## Scope

- Spec sections: § 6.17.2 (mirror :4638-4644). § 6.17.4.3 (mirror :5251-5258) restates the
  same four inequalities over the full `0..REFS_PER_FRAME-1` set — a superset; the validator
  models only the explicit map's `ref_frame_idx[0..NumTotalRefs]`, which is exactly the
  §6.17.2 index range.
- Crates/modules: `crates/splot-validate/src/context/reference_frames.rs`
  (`reference_state_checks`: one new block iterating `core.inter.ref_frame_idx`, gated on
  `Some(core.frame_size)` and `SlotState::Valid`).
- Diagnostics: new `frame-header/ref-frame-scale-ratio` (error, § 6.17.2).
- Docs: matrix row notes/diagnostics/proof; `VALIDATOR-DIAGNOSTICS.md` registration.

## Non-goals

- The `0..REFS_PER_FRAME-1` slots beyond `NumTotalRefs` (the implicit `get_ref_frames()` map)
  — unmodeled derivation, a named residual.
- Reference-state checks needing decoded `OrderHints`/`RefOrderHint` (RESTRICTED_OH), BRU
  `RefCounter` uniqueness, or ref-slot `MLayer/TLayerDependencyMap` — separate later slices.
- Any reconstruction / entropy-decode work. The check reads no new bits and stops cleanly at
  the frame-header parse boundary (`disable_cdf_update`, mirror :5041, before `tile_info()`).

## Acceptance criteria

- [ ] `frame-header/ref-frame-scale-ratio` registered and emitted from
      `reference_state_checks` only on `SlotState::Valid` slots with `core.frame_size` known.
- [ ] Negative: each of the four inequalities fires (width/height too small; width/height too
      large).
- [ ] Positive: a 1:1 ratio, the 2x-upscale boundary, an Unknown slot, and a ProvenInvalid
      slot all stay silent for this rule.
- [ ] Saturating arithmetic so a 16x/2x product overflow never invents a violation.
- [ ] `cargo xtask ci` passes.
