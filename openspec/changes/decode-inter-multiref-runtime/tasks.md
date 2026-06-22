# Tasks

## 1. OpenSpec and Feature Tracking
- [x] 1.1 Validate the `decode-inter-multiref-runtime` OpenSpec artifacts.
- [x] 1.2 Add `DECODE-INTER-MULTIREF-RUNTIME` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.3 Add the corresponding decoder support row; flip the
      `inter-single-ref-symbol` row note to WIRED.

## 2. Fixture (3-oracle verified)
- [x] 2.1 Generate a 3-frame single-reference stream (broad tools off,
      `--cdf-update-mode=0`) whose frame 2 § 7.7 ranks `ref_frame_idx [0, 1]`
      (NumTotalRefs == 2) and selects slot 1 (the retained inter frame), with
      frame 1 DISTINCT from the key so the selection is falsifiable.
- [x] 2.2 Confirm avmdec `--rawvideo --i420` == dav2d `--demuxer ivf --muxer yuv`
      byte-for-byte; record the md5 in `docs/LOCAL-REFERENCE-EVIDENCE.toml`.
- [x] 2.3 Commit the fixture + the manifest entry (`expect = "clean"`).

## 3. splot-core § 7.7 two-valid-slot feed
- [x] 3.1 Add `RefBaseQIdx` to `FrameReferenceStateView`
      (`from_slots_with_base_q_idx`); keep `from_slots` (no `RefBaseQIdx`)
      backward-compatible.
- [x] 3.2 Feed `RefBaseQIdx` into `derive_implicit_ref_map` and lift the
      `valid_count > 1` → `UnmodeledDerivation` gate ONLY when `RefBaseQIdx` is
      modeled; the historical view STAYS an `UnmodeledDerivation` stop.
- [x] 3.3 Unit-test the two-valid-slot derivation (`[0, 1]`) through the view and
      the unmodeled-without-base-q-idx stop.

## 4. splot-decode § 7.20 / § 7.23 reference retention
- [x] 4.1 Add `RuntimeReferenceBuffer`: the § 7.23 update per `refresh_frame_flags`
      (KEY/SWITCH: `RefValid[i] = first`; inter: `RefValid[i] = 1`), per-slot
      metadata + decoded-frame index, and a borrowed `ReferenceFrameStore` builder.
- [x] 4.2 Extend the multi-frame driver to decode a key + up to two inter frames
      (3 + 2·(N − 1) OBUs); apply the § 7.23 update after each frame.
- [x] 4.3 Unit-test the buffer (key marks only the first slot valid; an inter
      refresh adds a second valid slot).

## 5. splot-decode § 5.20.7.12 single_ref wiring
- [x] 5.1 Read `single_ref` between `read_skip` and `single_mode` when
      `NumTotalRefs == 2`; resolve the per-block reference via
      `ref_frame_idx[RefFrame[0]]`.
- [x] 5.2 Derive the § 8.3.2 `single_ref` context from the neighbour `count_refs`
      (`BlockNeighbourContext::single_ref_ctx`), cross-checked vs AVM
      `av2_get_ref_pred_context`; unit-test the no-neighbour ctx == 1.
- [x] 5.3 Relax the `NumTotalRefs == 1` gate to admit NumTotalRefs ∈ {1, 2}
      (single reference, non-compound) only.

## 6. Bit-exact decode + asymmetric retention proof
- [x] 6.1 `splot decode syn-3frame-multiref-64x64.ivf --output-format raw` ==
      avmdec == dav2d byte-for-byte (md5 861078138ab514bd847ccfe22ac44fa1).
- [x] 6.2 A reference-retention test: frame 2's luma equals frame 1's and DIFFERS
      from the key's (instrumented confirmation that frame 2 reads slot 1 via a
      real single_ref read).
- [x] 6.3 Per-frame decode-hash regression pins.

## 7. Verify + gate (verified-subset discipline)
- [x] 7.1 Negative tests: NumTotalRefs > 2 / compound / a 4th frame / a frame that
      would inherit an adapted inter frame's CDFs are all rejected before output.
- [x] 7.2 All existing inter and general-intra fixtures decode byte-identical.
- [x] 7.3 `cargo xtask ci` passes; `openspec validate --all` clean.

## 8. Deferred (out of scope, named follow-on)
- [ ] 8.1 § 7.23 cross-frame CDF save/load (so `--cdf-update-mode != 0` streams
      decode).
- [ ] 8.2 `NumTotalRefs > 2` / multi-decision `single_ref` and a neighbour-having
      `single_ref` context.
- [ ] 8.3 Compound references (`read_compound_ref`, § 5.20.7.11), temporal MV
      (ref-frame-mvs), and the deferred § 7.12.2 ref-MV-bank / DRL-reorder / warp
      candidates.
