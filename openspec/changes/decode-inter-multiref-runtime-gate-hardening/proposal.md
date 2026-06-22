## Why

The merged `DECODE-INTER-MULTIREF-RUNTIME` brick (PR #447) admits some streams its
verified subset does not actually decode correctly. Five review findings show the
admission gates can let a confident-but-wrong frame through (the committed fixtures
themselves decode bit-exact; the gaps are in what the gates ADMIT, not the committed
content):

1. The § 7.23 cross-frame CDF-load reject treated `PRIMARY_REF_CHOOSE` (the
   unsignalled `primary_ref_frame`) as "does not load". But AV2 § 5
   `set_primary_ref_frame_and_ctx` (mirror :5414-5415) RESOLVES `PRIMARY_REF_CHOOSE`
   to `DerivedPrimaryRefFrame` (a real reference) before `load_cdfs`. A CHOOSE frame
   resolving to a real ADAPTED inter reference therefore bypassed the guard and
   would decode from default CDFs → confident-wrong.
2. The CDF-adaptation flag collapsed to "any prior frame adapted", so a conformant
   frame loading a NON-adapted slot was wrongly rejected while the adapted check was
   imprecise.
3. `RefOrderHint` stored only `OrderHintLsbs`; an order-hint-wrapped history would
   feed a stale small value to the § 7.7 / `choose_primary_secondary_ref_frame`
   ranking → wrong `ref_frame_idx`.
4. A later inter frame using temporal MVs (`enable_ref_frame_mvs` /
   `use_ref_frame_mvs`) could draw § 7.12 temporal candidates from a prior frame's
   saved MVs, but the reference buffer stores no `SavedMvs`.
5. `derive_implicit_ref_map` admitted a multi-valid view whenever `ref_base_q_idx`
   was present, silently defaulting any missing `RefOrderHint` / dims to zero, so a
   caller with incomplete slices could derive `ref_frame_idx` from fabricated state.

## What Changes

- splot-decode: model § 5 `set_primary_ref_frame_and_ctx` PRECISELY, including the
  `PRIMARY_REF_CHOOSE` resolution via `choose_primary_secondary_ref_frame` (the
  inter-only `RefFrameType == INTER_FRAME` candidate filter, `qpDiff` scoring, and
  `is_ref_better` order-hint tie-break, cross-checked vs AVM). The cross-frame
  CDF-load reject now fires iff the RESOLVED `primary_ref_frame` loads a PER-SLOT
  ADAPTED reference slot. Each § 7.23 slot records `RefFrameType` and
  `disable_cdf_update` independently (replacing the coarse "any prior adapted" flag).
- splot-decode: reject (before output) a frame using temporal MVs once an INTER
  reference has been retained (no `SavedMvs` modeled), and a reference history whose
  order hints span a full `OrderHintBits` window (the stored `RefOrderHint` is the
  unwrapped `OrderHint` only within one window).
- splot-core: harden `derive_implicit_ref_map` so the `valid_count > 1`
  `UnmodeledDerivation` stop holds unless ALL § 7.7 ranking inputs are supplied as
  complete parallel slices covering every active slot (plus the current frame size
  when `check_res`).

## Capabilities

### Modified Capabilities
- `decode-inter-multiref-runtime`: the verified-subset admission gates are tightened
  so a stream that would inherit an adapted slot's CDFs (after the precise
  `PRIMARY_REF_CHOOSE` resolution), use temporal MVs over a retained inter
  reference, present an order-hint-wrapped history, or supply an incomplete reference
  state is rejected before output rather than confidently mis-decoded.

## Impact

- splot-decode: `RuntimeReferenceBuffer` slots gain per-slot `is_inter` / `adapted`;
  the inter decode resolves the CDF load and applies the per-slot reject; two new
  rejects (`inter_temporal_mvs_unmodeled`, `inter_order_hint_wrapped`) and the
  reworded `inter_cdf_inheritance_unmodeled`. No new diagnostics class (still
  `decode/unsupported-feature`).
- splot-core: `derive_implicit_ref_map` completeness gate; no public API change.
- ZERO regression: every committed inter and general-intra fixture stays
  byte-identical (re-decoded; md5s unchanged).
- Still out of scope (named follow-on): § 7.23 cross-frame CDF save/load (so an
  adapted reference can actually be LOADED instead of rejected), § 7.23 `SavedMvs`
  (temporal MV prediction), storing the unwrapped `OrderHint` (so a wrapped history
  decodes), `NumTotalRefs > 2`, compound references, and the deferred § 7.12.2
  candidates.
