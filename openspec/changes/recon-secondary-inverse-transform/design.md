## Context

The roadmap deferred § 7.15.3 as "entangled" because deriving the IST `kernel`
and `transpose` pulls in `YMode`, `pAngle`, `wide_angle_mapping`,
`most_probable_stx_set`, and `PlaneTxType`. But the § 7.15.3 process body splits
cleanly into that derivation and a pure matrix transform; the latter is the same
shape as the other `splot-recon` transform primitives, which take caller-resolved
`txSz`-derived values.

## Goals / Non-Goals

**Goals:**

- A total, panic-free `splot-recon` primitive for the § 7.15.3 matrix transform
  over caller-resolved facts.
- Reuse the existing `splot-tables` IST kernels / `Stx_Scan_Map` and the
  `coefficient_scan_order` 2D scan; hand-write only the two spec-inline
  `Stx_Scan_Order` constants.

**Non-Goals:**

- The § 7.15.3 `kernel` / `transpose` / `n` derivation from block state, and any
  runtime wiring.

## Decisions

- **Caller resolves `kernel`, `transpose`, `n`.** These need `YMode`,
  `AngleDeltaY`, `MrlIndex`, `most_probable_stx_set`, `is_inter`, and
  `PlaneTxType` — block/frame state `splot-recon` does not hold. Passing them as
  scalars matches the crate-wide contract and keeps the primitive free of that
  state.
- **Take `w` / `h`; derive `large`, `bwl`, the output width, and `scanW` /
  `scanBwl` internally.** `bwl == log2(w)` because `w = Min(32, Tx_Width)` and
  `bwl = Min(5, Tx_Width_Log2)` agree for every shape, so one `(w, h)` source
  determines all the geometry — no dual-source hazard.
- **Hand-write `Stx_Scan_Order_4x4` / `Stx_Scan_Order_8x8`.** They are § 7.15.3
  process-body constants absent from `all_tables.h`, so they are hand-written and
  spec-cited like `Transform_Shift` and `Transform_1d_Type`. The IST kernels and
  `Stx_Scan_Map` *are* in `all_tables.h`, so those come from the generated
  `splot-tables` copies.
- **i64 accumulation, validated indices.** At most 32 products of
  `Clip3`-bounded coefficients (`|c| < 2^17` at 10-bit) and small kernel weights
  (`< 2^8`) sum to well under `i64`; `n` / `kernel` / `sec_tx_type` are validated
  against the selected kernel set before any index is used, so the primitive is
  total and fail-atomic.
- **`sec_tx_type` is `1..=3`.** `sec_tx_type == 0` means "no secondary
  transform"; the caller gates on that, and the primitive rejects 0 (and indexes
  the kernel at `sec_tx_type - 1`).

## Risks / Trade-offs

- The in-test re-trace reference mirrors the production logic, so the
  small/large reference-match tests are not fully independent. The
  `small_4x4_dc_only_matches_hand_computed_kernel_values` test is the
  independent anchor: it pins three outputs (102, -45, -53) computed by hand from
  the literal `IST_4X4_KERNEL` weights, so a transcription or wiring error in the
  production matrix multiply / scatter / `Round2Signed` is caught regardless of
  the reference.
- It is loaded ahead of its runtime caller, matching the established pattern of
  building the § 7.15 transforms before the runtime wiring; the matrix and
  roadmap keep the kernel/transpose derivation and decode wiring out of scope.
