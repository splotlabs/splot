# Tasks: decode-inter-header-shared-tail

## 1. OpenSpec And Feature Scope

- [x] 1.1 Validate `decode-inter-header-shared-tail` with strict OpenSpec checks.
- [x] 1.2 Add Feature ID `DECODE-INTER-HEADER-SHARED-TAIL` to the matrix and
      reference it from `AV2-5.18.2-FRAME-HEADER-INFO`; document the verified
      minimal-tool single-reference subset and the deferred inter arms
      (segmentation-on, lr temporal / ccso reuse, `use_global_motion` warp models,
      the TIP / bridge return arms).

## 2. Sequence-View Plumbing

- [x] 2.1 Add `enable_df_sub_pu` to `CoreSeqFilterView` and `enable_bawp` /
      `enable_global_motion` to `CoreSeqInterView`; populate them in
      `CoreSeqView::from_sequence` and the `new_minimal_intra` encoder-input
      constructors.
- [x] 2.2 Add the `read_allow_df_sub_pu` parameter to
      `parse_deblocking_filter_params` (the §5.18.5.2 inter `allow_df_sub_pu` arm,
      mirror :5935); thread `false` through every intra / switch / writer / test
      caller and `true` (gated on `enable_df_sub_pu && FrameType == INTER_FRAME`)
      on the inter path.

## 3. Shared-Tail Parser

- [x] 3.1 Add `crates/splot-core/src/headers/frame/inter_shared_tail.rs` with the
      crate-private `parse_inter_shared_tail` reusing the intra sub-parsers + the
      inter arms, the admission gate (restoration-temporal / ccso-reuse), and the
      `InterTail` struct.
- [x] 3.2 Register the module in `crates/splot-core/src/headers/frame/mod.rs`.
- [x] 3.3 Add `FrameHeaderParseStatus::InterHeaderComplete` and the
      `FrameHeaderCore::inter_tail` field.
- [x] 3.4 Add `SegmentationParams::disabled()` and read `segmentation_enabled`
      inline in the shared tail (gate the unmodeled enabled-segmentation inter arm).
- [x] 3.5 Wire `parse_inter_path` to continue into the shared tail on
      `ReachedSharedTail` via `finish_inter_control_with_tail` (lift the
      reference-grounded frame size onto `core` before the tail; convert an EOF in
      the tail to the facts-preserving `StoppedInsideInterControl`).

## 4. Tests

- [x] 4.1 Bit-level fixture test: drive `parse_frame_header_core` on the real
      `syn-2frame-inter-64x64.ivf` inter-frame bytes and assert
      `InterHeaderComplete` with the expected parsed values (tile count, base_q_idx
      119, segmentation off, deblocking off, tx_mode largest, reference_select 0,
      skip_mode_present 0, allow_bawp/warpmv off, reduced_tx_set 0,
      use_global_motion off, apply_grain off).
- [x] 4.2 Focused completion test with ASYMMETRIC inter-tail values
      (reference_select == 1, skip_mode_present == 1, tx_mode_select == 1) so an
      adjacent-f(1) swap is caught.
- [x] 4.3 Honest-stop tests: segmentation-enabled stops before the enabled block;
      ccso-enabled stops at the admission gate before any shared-tail bit.
- [x] 4.4 Deblocking inter `allow_df_sub_pu` arm unit tests (reads-first alignment,
      and intra-path skips the bit) + the never-panics proptest input.
- [x] 4.5 Update the existing `frame_header_core_inter_explicit_map_reaches_shared_tail`
      test: its payload ends at the shared-tail boundary, so it now continues into
      the tail and truncates honestly (`StoppedInsideInterControl`).

## 5. Docs And Gate

- [x] 5.1 Extend the `AV2-5.18.2-FRAME-HEADER-INFO` matrix notes + proof/commands.
- [x] 5.2 Regenerate the four generated coverage docs.
- [x] 5.3 `cargo xtask ci` bare → "ci: all checks passed";
      `openspec validate --all --no-interactive` clean; zero deletions.
