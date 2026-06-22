# Tasks

## 1. OpenSpec and Feature Tracking
- [x] 1.1 Add this follow-up change and validate the OpenSpec artifacts.
- [x] 1.2 Update the `DECODE-INTER-MULTIREF-RUNTIME` matrix notes + proof list to
      record the hardened gates.

## 2. P1/P2 — Resolve PRIMARY_REF_CHOOSE + per-slot CDF adaptation
- [x] 2.1 Add per-slot `is_inter` (`RefFrameType == INTER_FRAME`) and `adapted`
      (`disable_cdf_update == 0`) to `RuntimeReferenceBuffer` and thread them into
      `InterReferenceState`.
- [x] 2.2 Model § 5 `set_primary_ref_frame_and_ctx` (`resolve_cdf_load`) including
      the `PRIMARY_REF_CHOOSE` resolution via `choose_primary_secondary_ref_frame`
      (`choose_primary_ref_frame`, inter-only candidate filter + `qpDiff` /
      `is_ref_better`), cross-checked vs AVM `av2/common/pred_common.c`.
- [x] 2.3 Apply the cross-frame CDF-load reject against the RESOLVED loaded slot's
      per-slot `adapted` flag (`inter_cdf_inheritance_unmodeled`).
- [x] 2.4 Unit-test `choose_primary_ref_frame` (skips non-inter slots, ranks by
      `qpDiff`) and `resolve_cdf_load` (CHOOSE→NONE→Default, CHOOSE→inter→LoadSlot,
      init-disabled→Default, explicit primary).

## 3. P2 — Order-hint wrap guard
- [x] 3.1 Add `order_hint_history_unwrapped`; reject a history spanning a full
      `OrderHintBits` window (`inter_order_hint_wrapped`).
- [x] 3.2 Unit-test the wrap guard (small monotonic = ok, full window = reject).

## 4. P2 — Temporal-MV reject after retaining an inter reference
- [x] 4.1 Reject `enable_ref_frame_mvs` / `use_ref_frame_mvs` once an inter
      reference is retained (`inter_temporal_mvs_unmodeled`).

## 5. P2 — Complete reference state before multi-ref ranking
- [x] 5.1 Harden `derive_implicit_ref_map`: keep the `valid_count > 1`
      `UnmodeledDerivation` stop unless all § 7.7 ranking-input slices are complete
      (cover every active slot) and the frame size is present when `check_res`.
- [x] 5.2 Unit-test an incomplete (short-slice) two-valid-slot view stops
      `UnmodeledDerivation` even with `RefBaseQIdx` present.

## 6. Verification
- [x] 6.1 Re-decode every committed inter + general-intra fixture; confirm
      byte-identical (zero regression).
- [x] 6.2 `cargo xtask ci` exits 0; `openspec validate --all` passes.
