## 1. Tracking

- [x] 1.1 Add `DECODE-FIRST-INTER-FRAME-FRONTIER` to the implementation matrix.
- [x] 1.2 Add the decoder support row for `first-inter-frame-frontier`.
- [x] 1.2a The decoder conformance coverage now reflects real inter decode: the inter frontier's spec sections (§5.18.2, §5.20, §7.11, §7.13.3.18, §7.23) map to the existing `frame-header-state` / `tile-group-and-payload-syntax` / `prediction-process` / `reference-frame-management` coverage rows, regenerated from the updated `first-inter-frame-frontier` support row (no longer overstating, since the inter frame decodes bit-exact).
- [x] 1.3 Add the `syn-2frame-inter-64x64.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Fixture verification

- [x] 2.1 Generate `syn-2frame-inter-64x64.ivf` locally from a project-owned flat synthetic Y4M with broad decode tools disabled and `--enable-global-motion=1 --qp=80 --sb-size=64 --min/max-partition-size=64`.
- [x] 2.2 Confirm via `splot inspect` the OBU shape: frame 0 = TD + SEQUENCE_HEADER + CLOSED_LOOP_KEY, frame 1 = TD + REGULAR_TILE_GROUP.
- [x] 2.3 Confirm `avmdec --rawvideo --i420` equals `dav2d --demuxer ivf` byte-for-byte (decoded-output md5 `4e1bd39f0b541ef1f479cff049e6985c`, 12288 bytes) and that frame 1 == a copy of frame 0 (zero-MV skip inter, no residual).
- [x] 2.4 Confirm the fixture validates clean.

## 3. Honest current state

- [x] 3.1 The prior honest rejection (`decode/unsupported-feature` at the planner, then at the inter frame) is now superseded: `splot decode --output-format raw` on the inter fixture decodes BOTH frames bit-exact (whole-stream md5 4e1bd39f0b541ef1f479cff049e6985c), pinned by `decode_two_frame_inter_fixture_decodes_both_frames_bit_exact`.
- [x] 3.2 Confirm all existing intra fixtures still decode bit-exact (no regression).

## 4. Inter decode slice (the full INTER arc — LANDED, bit-exact)

- [x] 4.1 Model the § 7.7 `get_ref_frames()` implicit reference-map derivation (already modeled in `splot-core`); the runtime supplies a one-valid-slot `FrameReferenceStateView` so the § 5.18.2 inter parse resolves NumTotalRefs == 1 / ref_frame_idx == [0] exactly (no longer stopping at `InterStop::UnmodeledDerivation`).
- [x] 4.2 Continue the § 5.18.2 inter frame-header shared tail to InterHeaderComplete (tile_info → quantization → segmentation → filters → frame_reference_mode → skip_mode_params → global_motion_params → film_grain); already modeled in `splot-core`, now driven from the runtime with the post-key reference state.
- [x] 4.3 The stream planner accepts the inter OBU_REGULAR_TILE_GROUP as a second frame candidate; the multi-frame runtime loop decodes the key frame then the inter frame and emits both.
- [x] 4.4 Retain the decoded key as a reference (§ 7.23) in a splot-recon `ReferenceFrameStore<&DecodedFrame<u8>>` (zero-copy borrow).
- [x] 4.5 Decode the § 5.20.7.6 inter mode_info for one block (is_inter / skip / single_mode / read_drl_idx; read_skip_mode / single_ref / read_motion_mode read no symbols in this subset) consuming new inter CDFs (is_inter / skip / single_mode / drl_mode), gated to the verified single-reference zero-MV skip subset and rejecting everything else with structured diagnostics. The actual mode is NEARMV (single_mode == 0).
- [x] 4.6 Derive MV = (0, 0) for the zero-MV NEARMV/GLOBALMV case (§ 7.11) with an `Mv` newtype.
- [x] 4.7 Run § 7.13.3.18 zero-fraction motion compensation (copy the co-located reference planes) and wire frame 1 to output, proving bit-exact equality to avmdec == dav2d (whole-stream md5 4e1bd39f0b541ef1f479cff049e6985c) guarded by § 8.2.4 exit_symbol().

## 5. Documentation And Verification

- [x] 5.1 Regenerate feature/status/coverage docs.
- [x] 5.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, reference evidence, and the Rust acceptance gate.
