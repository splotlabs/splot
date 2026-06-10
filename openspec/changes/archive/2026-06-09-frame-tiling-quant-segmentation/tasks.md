# Tasks: frame header tiling, quantization, and segmentation

## 1. Spec mapping and state plumbing

- [x] 1.1 Update `docs/SPEC-MAPPING.md` for § 5.18.6.1–§ 5.18.6.3, § 5.18.7.1,
      § 5.18.7.2, § 5.18.7.8, and the § 5.18.2 lossless/`allow_tcq`/
      `allow_parity_hiding` tail, citing mirror paths (required before bitstream
      changes).
- [x] 1.2 Extend `CoreSeqView` (grouped quant/seg/tile sub-views per design
      decision 2) with the sequence-derived inputs the new structures need, wired
      from the parsed sequence header in `splot-validate` context.

## 2. Quantization parsing (`AV2-5.18.6-QUANTIZATION`)

- [x] 2.1 Implement `read_delta_q()` (§ 5.18.6.3) and `quantization_params()`
      (§ 5.18.6.1) in `crates/splot-core/src/headers/frame/quant.rs` with typed
      output fields; positive, negative, and EOF tests (incl. monochrome,
      shared-UV-delta, equal_ac_dc_q, and 9-bit `base_q_idx` cases).
- [x] 2.2 Implement `setup_qm_params()` (§ 5.18.6.2) with tests for
      using_qmatrix=0, segmentation-gated `pic_qm_num_minus_1`, and
      `qm_uv_same_as_y`/`separate_uv_delta_q` gating.
- [x] 2.3 Implement `delta_q_params()` (§ 5.18.7.8) and the § 5.18.2 per-segment
      lossless/QM derivation (`get_qindex(1, segId)` minimal form, `LosslessArray`,
      `CodedLossless`, `qm_index` reads, `allow_tcq`, `allow_parity_hiding`) with
      hand-computed test vectors from the mirror text.

## 3. Tiling and segmentation parsing (`AV2-5.18.7-SEGMENTATION-TILING`)

- [x] 3.1 Implement `tile_info()` (§ 5.18.7.2) in
      `crates/splot-core/src/headers/frame/tiling.rs`, deriving `MiCols`/`MiRows`
      via `compute_image_size()` (§ 5.18.4.4), reusing `tile.rs` helpers
      (`tile_params`, `reuse_tile_params`, `uniform_eligible`); tests for
      single-tile, explicit multi-tile, reuse path, context-update gating, and EOF.
- [x] 3.2 Implement `segmentation_params()` (§ 5.18.7.1) in
      `crates/splot-core/src/headers/frame/segmentation.rs`, reusing
      `seg_info()`; derive `SegIdPreSkip`/`LastActiveSegId`; intra-path inference
      of `segmentation_update_map`/`segmentation_temporal_update`; tests for
      disabled, fresh seg_info, sequence-reuse, and EOF cases.
- [x] 3.3 Wire the new structures into `parse_intra_tail` in exact § 5.18.2 order;
      replace `StoppedBeforeFilteringQuantSegmentation` with
      `StoppedBeforeDeblockingFilterParams`; explicit
      `UnsupportedUntilFeature` stops for unmodeled MFH-gated branches
      (`cur_mfh_id > 0`); update/extend property tests so all new paths are
      covered by no-panic proptests.

## 4. Validator diagnostics and inspector

- [x] 4.1 Add § 6.17.7.2 diagnostics (TileCols/TileRows bounds,
      `context_update_tile_id` range) and § 6.17.6.2 custom-QM plane-count checks
      (gated on available QM state, no false positives) in `splot-validate`;
      register new rule ids in `docs/VALIDATOR-DIAGNOSTICS.md`; positive and
      negative diagnostic tests.
- [x] 4.2 Surface the new quantizer/QM/segmentation/tile fields and the new parse
      status label in the `splot inspect` JSON frame-header summary; update
      snapshot tests; confirm existing diagnostics are unchanged.

## 5. Docs, matrix, and gates

- [x] 5.1 Update `docs/IMPLEMENTATION-MATRIX.toml` (`AV2-5.18.6-QUANTIZATION`,
      `AV2-5.18.7-SEGMENTATION-TILING`, umbrella rows stay `partial`) with proof,
      regenerate `docs/FEATURE-STATUS.md`, and add the Phase 8 status note to
      `docs/VALIDATOR-ROADMAP.md`.
- [x] 5.2 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`,
      `cargo xtask ci`, and `cargo xtask audit-scope --all --write-ledger`; fix
      all findings.
