## 1. Tracking

- [x] 1.1 Add `DECODE-FIRST-INTER-FRAME-FRONTIER` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `first-inter-frame-frontier`.
- [x] 1.3 Add the `syn-2frame-inter-64x64.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Fixture verification

- [x] 2.1 Generate `syn-2frame-inter-64x64.ivf` locally from a project-owned flat synthetic Y4M with broad decode tools disabled and `--enable-global-motion=1 --qp=80 --sb-size=64 --min/max-partition-size=64`.
- [x] 2.2 Confirm via `splot inspect` the OBU shape: frame 0 = TD + SEQUENCE_HEADER + CLOSED_LOOP_KEY, frame 1 = TD + REGULAR_TILE_GROUP.
- [x] 2.3 Confirm `avmdec --rawvideo --i420` equals `dav2d --demuxer ivf` byte-for-byte (decoded-output md5 `4e1bd39f0b541ef1f479cff049e6985c`, 12288 bytes) and that frame 1 == a copy of frame 0 (zero-MV skip inter, no residual).
- [x] 2.4 Confirm the fixture validates clean.

## 3. Honest current state

- [x] 3.1 Pin the honest rejection: `splot decode --output-format raw` on the inter fixture returns `decode/unsupported-feature` (OBU_REGULAR_TILE_GROUP not yet a frame candidate, § 5.2.1) with NO output, via `decode_two_frame_inter_fixture_is_rejected_at_planner_today`.
- [x] 3.2 Confirm all existing intra fixtures still decode bit-exact (no regression).

## 4. Inter decode slice (deferred — the full INTER arc)

- [ ] 4.1 (deferred) Model the § 7.7 `get_ref_frames()` implicit reference-map derivation in `splot-core` so the § 5.18.2 inter frame-header parse resolves NumTotalRefs / ref_frame_idx[] (the empirically-established blocker; the parser stops at `InterStop::UnmodeledDerivation`).
- [ ] 4.2 (deferred) Continue the § 5.18.2 inter frame-header shared tail (tile_info → quantization → segmentation → filters → frame_reference_mode → skip_mode_params → global_motion_params → film_grain).
- [ ] 4.3 (deferred) Relax the stream planner (`classify_obu`) + minimal-tier shape gates to accept the inter OBU_REGULAR_TILE_GROUP as a second frame candidate, and add a multi-frame runtime loop emitting N frames.
- [ ] 4.4 (deferred) Retain the decoded key as a reference (§ 7.23) in a splot-recon `ReferenceFrameStore`.
- [ ] 4.5 (deferred) Decode the § 5.20 inter mode_info for one block (skip_mode / is_inter / skip / ref_frames / single-ref GLOBALMV-NEARESTMV / DRL) consuming the inter CDFs, gated to the verified zero-MV skip subset and rejecting everything else with structured diagnostics.
- [ ] 4.6 (deferred) Derive MV = (0, 0) for the GLOBALMV/NEARESTMV-zero case (§ 7.11) with an `Mv` newtype.
- [ ] 4.7 (deferred) Run § 7.13.3.18 zero-fraction motion compensation (copy the co-located block from the reference) and wire frame 1 to output, proving bit-exact equality to avmdec == dav2d and pinning the frame hash.

## 5. Documentation And Verification

- [x] 5.1 Regenerate feature/status/coverage docs.
- [x] 5.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, reference evidence, and the Rust acceptance gate.
