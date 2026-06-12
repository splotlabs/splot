# Tasks: § 5.18.2 inter/TIP/bridge/switch control region

## 1. Bookkeeping

- [x] 1.1 Confirm matrix row ids; `openspec_change` set; re-read the
  full non-intra § 5.18.2 region verbatim (05 mirror #s-5-18-2), plus
  § 5.18.3.1/.2, § 5.18.4.2/.3, § 5.18.5.1, and the § 6.17.2 semantics
  for the new fields. (Row id confirmed `AV2-5.18.3-FRAME-CONFIGURATION`;
  `openspec_change = frame-header-inter-reference-paths` on the 5.18.2 row.)

## 2. Parsing

- [x] 2.1 Primary-ref signaling + inter refresh branches (incl. bridge
  overwrite). (inter.rs: signal_primary_ref_frame / disable_cross_frame_cdf_init /
  primary_ref_frame f(3) / PRIMARY_REF_CHOOSE / SWITCH+bridge PRIMARY_REF_NONE;
  bridge_frame_overwrite_flag; inter/switch/short refresh arms; RAS arm honest stop.)
- [x] 2.2 Explicit reference map + ref_frame_idx; BRU triple; ref-mvs/
  tmvp; TIP block; DRL; MV precision; motion modes. (frame_explicit_ref_frame_map,
  num_total_refs f(3), ref_frame_idx[i]; use_bru/bru_ref/bru_inactive; use_ref_frame_mvs /
  tmvp_sample_step_minus_1; TIP gate stops honestly at usesEqualWeight; change_drl /
  max_drl_bits_minus_1 ns(n); use_qtr_precision_mv / allow_high_precision_mv;
  frame_enabled_motion_modes[mode].)
- [x] 2.3 read_interpolation_filter; df_sub_pu/TIP deblocking;
  frame_size_with_refs/_with_bridge; § 5.18.3 derivations. (read_interpolation_filter()
  in filtering.rs; frame_size_with_refs/_with_bridge in inter.rs; frame_opfl_refine_type()
  in inter.rs. allow_df_sub_pu / apply_deblocking_filter_tip live on the TIP-output arm,
  which stops honestly (TipAsOutputReturn) — named residual.)
- [x] 2.4 Converge into the shared tail; § 5.19 BRU arms become
  decidable; honest stops on poisoned reference state; EOF/truncation
  per the partition; arithmetic audit. (InterStop::ReachedSharedTail at mirror :5183;
  poisoned/unmodeled stops are COVERAGE stops, never truncations; ns(n) widths /
  CeilLog2 bounds audited. The §5.19 BRU arms stay blocked: they need the inter frame to
  COMPLETE through the shared tail (NumFrameHeaderBits known), which awaits the inter
  shared-tail inputs — named residual in the matrix.)

## 3. Validation, surfacing, docs

- [x] 3.1 § 6 diagnostics with citations (ref-idx validity, bounds,
  primary-ref) or named residuals; inspect surfaces the region;
  fixtures; matrix proof; generated docs; roadmap.
  (frame-header/ref-frame-idx-invalid-slot §6.17.2 — proven-invalid + out-of-range;
  inspect `inter` view; VALIDATOR-DIAGNOSTICS.md + matrix rows updated; SPEC-COVERAGE.md
  regenerated. primary_ref_frame/NumTotalRefs bounds need unmodeled get_ref_frames() —
  named residuals. No new fixture: the end-to-end inter path is exercised by the in-tree
  reference-state validator tests built from the existing sequence helpers.)

## 4. Verification

- [x] 4.1 Per-structure positive/negative/EOF; honest-stop tests;
  proptests. (inter.rs unit tests + info.rs inter tests + validator ref_state_inter tests;
  the never-panic proptest exercises an arbitrary inter sequence view.)
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes. (CI_EXIT=0, all checks passed.)
